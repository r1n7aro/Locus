use fastembed::{
    EmbeddingModel as FastembedModel, ExecutionProviderDispatch, InitOptionsUserDefined, Pooling,
    QuantizationMode, TextEmbedding, TextInitOptions, TokenizerFiles, UserDefinedEmbeddingModel,
};
use hf_hub::api::sync::{ApiBuilder, ApiRepo};
use hf_hub::{Cache, Repo, RepoType};
use http::Uri;
use hyper_util::client::proxy::matcher::Matcher;
#[cfg(windows)]
use ndarray::{s, Array, Array2, ArrayView, Axis, Dim, Dimension, IxDynImpl};
#[cfg(windows)]
use ort::ep::{DirectML, ExecutionProvider, CUDA};
#[cfg(windows)]
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    tensor::TensorElementType,
    value::Value,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
#[cfg(windows)]
use std::sync::OnceLock;
use std::time::Instant;
#[cfg(windows)]
use tokenizers::{AddedToken, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};
use url::Url;
use walkdir::WalkDir;

const LOCAL_RUNTIME_FASTEMBED: &str = "fastembed";
const LOCAL_EMBED_BATCH_SIZE: usize = 64;
const LOCAL_EMBED_MAX_LENGTH: usize = 512;
const DEVICE_POLICY_CPU_FASTEMBED: &str = "cpu_fastembed";
const DEVICE_POLICY_GPU_DIRECTML: &str = "gpu_directml";
const DEVICE_POLICY_GPU_CUDA: &str = "gpu_cuda";
const LOCAL_TOKENIZER_FILE_NAMES: [&str; 4] = [
    "tokenizer.json",
    "config.json",
    "special_tokens_map.json",
    "tokenizer_config.json",
];
const REQUIRED_LOCAL_TOKENIZER_FILE_NAMES: [&str; 3] =
    ["tokenizer.json", "config.json", "tokenizer_config.json"];
const OPTIONAL_LOCAL_TOKENIZER_FILE_NAMES: [&str; 1] = ["special_tokens_map.json"];
const OPTIONAL_LOCAL_MODEL_FILE_NAMES: [&str; 3] = [
    "modules.json",
    "sentence_bert_config.json",
    "1_Pooling/config.json",
];
const OPTIONAL_LOCAL_MODEL_SENTENCE_TRANSFORMER_FILES: [&str; 1] =
    ["config_sentence_transformers.json"];
const LOCAL_MODEL_FILE_CANDIDATES: [&str; 6] = [
    "model.onnx",
    "onnx/model.onnx",
    "model_optimized.onnx",
    "onnx/model_optimized.onnx",
    "model_quantized.onnx",
    "onnx/model_quantized.onnx",
];
const LOCAL_MODEL_METADATA_FILE_NAME: &str = ".locus-model.json";
const LOCAL_MODEL_DOWNLOAD_SOURCE_OFFICIAL: &str = "official";
const LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR: &str = "hf-mirror";
const HF_ENDPOINT_OFFICIAL: &str = "https://huggingface.co";
const HF_ENDPOINT_HF_MIRROR: &str = "https://hf-mirror.com";
const EMPTY_JSON_OBJECT: &[u8] = br#"{}"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingBackendSignature {
    pub runtime_name: String,
    pub model_id: String,
    pub model_revision: String,
    pub device_route: String,
    pub dimension: usize,
    pub normalize: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_embedding_mode")]
    pub embedding_mode: String,
    #[serde(default = "default_device_policy")]
    pub device_policy: String,
    #[serde(default = "default_local_runtime")]
    pub local_runtime: String,
    #[serde(default = "default_local_model")]
    pub local_model: String,
    #[serde(default)]
    pub local_model_path: String,
    #[serde(default = "default_local_model_download_source")]
    pub local_model_download_source: String,
    #[serde(default)]
    pub remote_endpoint: String,
    #[serde(default)]
    pub remote_api_key: String,
    #[serde(default)]
    pub remote_model: String,
    #[serde(default)]
    pub remote_dimensions: u32,
    #[serde(default)]
    pub remote_max_batch: u32,
}

fn default_embedding_mode() -> String {
    "local".to_string()
}

fn default_device_policy() -> String {
    DEVICE_POLICY_CPU_FASTEMBED.to_string()
}

fn default_local_runtime() -> String {
    LOCAL_RUNTIME_FASTEMBED.to_string()
}

fn default_local_model() -> String {
    "Qwen/Qwen3-Embedding-4B".to_string()
}

fn default_local_model_download_source() -> String {
    LOCAL_MODEL_DOWNLOAD_SOURCE_OFFICIAL.to_string()
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            embedding_mode: default_embedding_mode(),
            device_policy: default_device_policy(),
            local_runtime: default_local_runtime(),
            local_model: default_local_model(),
            local_model_path: String::new(),
            local_model_download_source: default_local_model_download_source(),
            remote_endpoint: String::new(),
            remote_api_key: String::new(),
            remote_model: String::new(),
            remote_dimensions: 0,
            remote_max_batch: 0,
        }
    }
}

fn normalize_device_policy(device_policy: &str) -> &'static str {
    match device_policy.trim() {
        DEVICE_POLICY_CPU_FASTEMBED => DEVICE_POLICY_CPU_FASTEMBED,
        DEVICE_POLICY_GPU_DIRECTML => DEVICE_POLICY_GPU_DIRECTML,
        DEVICE_POLICY_GPU_CUDA => DEVICE_POLICY_GPU_CUDA,
        "cpu" | "cpu_only" => DEVICE_POLICY_CPU_FASTEMBED,
        "gpu" | "gpu_only" | "gpu_preferred" => {
            #[cfg(windows)]
            {
                DEVICE_POLICY_GPU_DIRECTML
            }
            #[cfg(not(windows))]
            {
                DEVICE_POLICY_CPU_FASTEMBED
            }
        }
        _ => DEVICE_POLICY_CPU_FASTEMBED,
    }
}

fn sanitize_embedding_config(mut config: EmbeddingConfig) -> EmbeddingConfig {
    config.device_policy = normalize_device_policy(&config.device_policy).to_string();
    config.local_model_download_source =
        normalize_local_model_download_source(&config.local_model_download_source).to_string();
    config
}

fn normalize_local_model_download_source(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "hf-mirror" | "mirror" | "cn" | "china" => LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR,
        _ => LOCAL_MODEL_DOWNLOAD_SOURCE_OFFICIAL,
    }
}

fn download_source_endpoint(download_source: &str) -> &'static str {
    match normalize_local_model_download_source(download_source) {
        LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR => HF_ENDPOINT_HF_MIRROR,
        _ => HF_ENDPOINT_OFFICIAL,
    }
}

fn local_model_download_source_label(value: &str) -> &'static str {
    match normalize_local_model_download_source(value) {
        LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR => "HF-Mirror",
        _ => "Hugging Face",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelPreset {
    pub id: String,
    pub label: String,
    pub downloaded: bool,
    pub dimensions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingAvailableLocalModel {
    pub model_id: String,
    pub label: String,
    pub local_model_path: String,
    pub dimensions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingLocalModelCatalog {
    pub managed_directory: String,
    pub presets: Vec<EmbeddingModelPreset>,
    pub available_models: Vec<EmbeddingAvailableLocalModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingLocalModelDirectoryInspection {
    pub path: String,
    pub label: String,
    pub ready: bool,
    pub model_file: Option<String>,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedLocalModelMetadata {
    model_id: String,
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pooling: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ManualPoolingMode {
    #[default]
    Cls,
    Mean,
    LastToken,
}

impl ManualPoolingMode {
    fn as_metadata_str(self) -> &'static str {
        match self {
            Self::Cls => "cls",
            Self::Mean => "mean",
            Self::LastToken => "last_token",
        }
    }
}

fn manual_pooling_from_metadata_str(value: &str) -> Option<ManualPoolingMode> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "cls" | "cls_token" => Some(ManualPoolingMode::Cls),
        "mean" | "mean_tokens" | "mean_sqrt_len_tokens" => Some(ManualPoolingMode::Mean),
        "last_token" | "lasttoken" => Some(ManualPoolingMode::LastToken),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbeddingPresetDownloadKind {
    Fastembed,
    HuggingFace,
}

#[derive(Debug, Clone, Copy)]
struct SupportedEmbeddingPreset {
    id: &'static str,
    label: &'static str,
    dimensions: usize,
    download_kind: EmbeddingPresetDownloadKind,
    download_model_id: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingDownloadNetworkStatus {
    pub source: String,
    pub endpoint: String,
    pub proxy_state: String,
    pub proxy_env_key: Option<String>,
    pub proxy_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingStatus {
    pub enabled: bool,
    pub ready: bool,
    pub activating: bool,
    pub model_downloaded: bool,
    pub model_download_progress: Option<f64>,
    pub index_progress: Option<f64>,
    pub error: Option<String>,
    pub stage: Option<String>,
    pub detail: Option<String>,
    pub current_file: Option<String>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub processed_docs: Option<usize>,
    pub total_docs: Option<usize>,
    pub failed_docs: Option<usize>,
    pub last_failed_file: Option<String>,
    pub last_failure: Option<String>,
    pub download_network: Option<EmbeddingDownloadNetworkStatus>,
    pub last_test_summary: Option<String>,
    pub last_test_passed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingRuntimeTestResult {
    pub passed: bool,
    pub summary: String,
    pub backend: String,
    pub model_id: String,
    pub dimension: usize,
    pub vector_count: usize,
    pub latency_ms: u64,
    #[serde(default)]
    pub cases: Vec<EmbeddingRuntimeTestCaseResult>,
    pub diagnostics: Option<EmbeddingRuntimeTestDiagnostics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingRuntimeTestCaseResult {
    pub case_id: String,
    pub route: String,
    pub provider: String,
    pub backend: String,
    pub model_id: String,
    pub dimension: usize,
    pub vector_count: usize,
    pub latency_ms: u64,
    pub outcome: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingRuntimeTestDiagnostics {
    pub dlls: Vec<EmbeddingRuntimeTestDllInfo>,
    pub adapters: Vec<EmbeddingRuntimeTestAdapterInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingRuntimeTestDllInfo {
    pub name: String,
    pub path: Option<String>,
    pub exists: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingRuntimeTestAdapterInfo {
    pub index: i32,
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub dedicated_vram_bytes: u64,
    pub is_software: bool,
    pub is_high_performance: bool,
}

#[derive(Debug, Clone)]
pub enum EmbeddingActivationProgress {
    Stage {
        stage: &'static str,
        detail: Option<String>,
    },
    Download {
        file_name: String,
        downloaded_bytes: u64,
        total_bytes: u64,
        progress: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingDownloadError {
    Cancelled,
    Failed(String),
}

impl EmbeddingDownloadError {
    fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

impl std::fmt::Display for EmbeddingDownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "Model download cancelled"),
            Self::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for EmbeddingDownloadError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectMlAdapterCandidate {
    index: i32,
    dedicated_video_memory: u64,
    is_software: bool,
    is_high_performance: bool,
}

fn prioritize_directml_adapter_indices(candidates: &[DirectMlAdapterCandidate]) -> Vec<i32> {
    let mut hardware_adapters: Vec<_> = candidates
        .iter()
        .filter(|candidate| !candidate.is_software)
        .cloned()
        .collect();

    // When Windows exposes a high-performance adapter, keep DirectML pinned to
    // that class of device. Integrated GPUs become a fallback only on systems
    // that do not expose a preferred high-performance adapter.
    hardware_adapters.sort_by(|left, right| {
        right
            .is_high_performance
            .cmp(&left.is_high_performance)
            .then_with(|| {
                right
                    .dedicated_video_memory
                    .cmp(&left.dedicated_video_memory)
            })
            .then_with(|| left.index.cmp(&right.index))
    });

    if hardware_adapters.is_empty() {
        return vec![0];
    }

    let preferred_adapters: Vec<_> = hardware_adapters
        .iter()
        .filter(|candidate| candidate.is_high_performance)
        .cloned()
        .collect();

    if !preferred_adapters.is_empty() {
        preferred_adapters
            .into_iter()
            .map(|candidate| candidate.index)
            .collect()
    } else {
        hardware_adapters
            .into_iter()
            .map(|candidate| candidate.index)
            .collect()
    }
}

#[cfg(windows)]
pub fn preload_ort_helpers() {
    static ORT_HELPERS_PRELOADED: OnceLock<()> = OnceLock::new();

    ORT_HELPERS_PRELOADED.get_or_init(|| {
        for dll in collect_runtime_helper_dlls() {
            tracing::info!(
                log_module = "knowledge_index",
                dll = %dll.name,
                path = %dll.path.clone().unwrap_or_default(),
                exists = dll.exists,
                version = %dll.version.clone().unwrap_or_default(),
                "embedding helper dll check"
            );
        }

        if let Some(path) = find_runtime_helper_dll("DirectML.dll") {
            if let Err(error) = ort::util::preload_dylib(&path) {
                tracing::warn!(
                    log_module = "knowledge_index",
                    path = %path.display(),
                    error = %error,
                    "failed to preload DirectML.dll"
                );
            }
        }
    });
}

#[cfg(not(windows))]
pub fn preload_ort_helpers() {}

#[cfg(windows)]
fn collect_runtime_helper_dlls() -> Vec<EmbeddingRuntimeTestDllInfo> {
    [
        "onnxruntime.dll",
        "onnxruntime_providers_shared.dll",
        "DirectML.dll",
    ]
    .into_iter()
    .map(|name| {
        let path = find_runtime_helper_dll(name);
        EmbeddingRuntimeTestDllInfo {
            name: name.to_string(),
            path: path.as_ref().map(|value| value.display().to_string()),
            exists: path.is_some(),
            version: path.as_deref().and_then(read_runtime_dll_version),
        }
    })
    .collect()
}

#[cfg(windows)]
fn find_runtime_helper_dll(file_name: &str) -> Option<PathBuf> {
    runtime_helper_search_dirs()
        .into_iter()
        .map(|dir| dir.join(file_name))
        .find(|candidate| candidate.is_file())
}

#[cfg(windows)]
fn runtime_helper_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            push_unique_path(&mut dirs, exe_dir);
            if exe_dir
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("deps"))
                .unwrap_or(false)
            {
                if let Some(parent) = exe_dir.parent() {
                    push_unique_path(&mut dirs, parent);
                }
            }
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        push_unique_path(&mut dirs, &current_dir);
    }

    dirs
}

#[cfg(windows)]
fn push_unique_path(paths: &mut Vec<PathBuf>, candidate: &Path) {
    if paths.iter().all(|path| path != candidate) {
        paths.push(candidate.to_path_buf());
    }
}

pub trait EmbeddingRuntime: Send + Sync {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String>;
    fn backend_signature(&self) -> EmbeddingBackendSignature;
    fn device_name(&self) -> Option<String> {
        None
    }
    fn gpu_memory_bytes(&self) -> Option<u64> {
        None
    }
}

pub struct EmbeddingManager {
    config: EmbeddingConfig,
    runtime: Option<Box<dyn EmbeddingRuntime>>,
    model_dir: PathBuf,
    last_error: Option<String>,
}

impl EmbeddingManager {
    pub fn new(config: EmbeddingConfig, model_storage_dir: &Path) -> Self {
        let config = sanitize_embedding_config(config);
        Self {
            config,
            runtime: None,
            model_dir: managed_model_root(model_storage_dir),
            last_error: None,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.config.enabled && self.runtime.is_some()
    }

    pub fn is_model_downloaded(&self) -> bool {
        if self.config.embedding_mode == "remote" {
            return true;
        }
        let requested_model = self.config.local_model.trim();
        let manual_model_dir = configured_local_model_dir(&self.config, &self.model_dir);
        if self.config.local_model_path.trim().is_empty() {
            if let Some(model) = fastembed_model_for_id(requested_model) {
                return fastembed_model_cached(
                    &effective_fastembed_cache_dir(&self.model_dir),
                    &model,
                );
            }
            if supported_embedding_preset(requested_model).is_some() {
                return manual_model_files_ready(&manual_model_dir);
            }
        }
        if manual_model_files_ready(&manual_model_dir) {
            return true;
        }
        if requested_model.is_empty() {
            return false;
        }
        false
    }

    pub fn model_dir_path(&self) -> PathBuf {
        configured_local_model_dir(&self.config, &self.model_dir)
    }

    pub fn managed_model_root_path(&self) -> &Path {
        &self.model_dir
    }

    pub fn config(&self) -> &EmbeddingConfig {
        &self.config
    }

    pub fn update_config(&mut self, config: EmbeddingConfig) -> bool {
        let next = sanitize_embedding_config(config);
        let restart_required = embedding_runtime_restart_required(&self.config, &next);
        self.config = next;
        if restart_required {
            self.runtime = None;
            self.last_error = None;
        }
        restart_required
    }

    pub fn status(&self) -> EmbeddingStatus {
        EmbeddingStatus {
            enabled: self.config.enabled,
            ready: self.is_ready(),
            activating: false,
            model_downloaded: self.is_model_downloaded(),
            model_download_progress: None,
            index_progress: None,
            error: self.last_error.clone(),
            stage: if self.is_ready() {
                Some("ready".to_string())
            } else {
                None
            },
            detail: None,
            current_file: None,
            downloaded_bytes: None,
            total_bytes: None,
            processed_docs: None,
            total_docs: None,
            failed_docs: None,
            last_failed_file: None,
            last_failure: None,
            download_network: None,
            last_test_summary: None,
            last_test_passed: None,
        }
    }

    pub fn activate_with_progress<F>(&mut self, on_progress: &mut F) -> Result<(), String>
    where
        F: FnMut(EmbeddingActivationProgress),
    {
        if !self.config.enabled {
            return Err("Embedding is not enabled in config".into());
        }

        on_progress(EmbeddingActivationProgress::Stage {
            stage: "preparing",
            detail: Some("Preparing embedding runtime".to_string()),
        });

        let runtime: Box<dyn EmbeddingRuntime> = match self.config.embedding_mode.as_str() {
            "remote" => {
                on_progress(EmbeddingActivationProgress::Stage {
                    stage: "initializing_runtime",
                    detail: Some("Connecting to remote embedding endpoint".to_string()),
                });
                match RemoteEmbeddingRuntime::new(&self.config) {
                    Ok(runtime) => Box::new(runtime),
                    Err(err) => {
                        self.last_error = Some(err.clone());
                        return Err(err);
                    }
                }
            }
            _ => match LocalEmbeddingRuntime::new_with_progress(
                &self.config,
                &self.model_dir,
                on_progress,
            ) {
                Ok(runtime) => Box::new(runtime),
                Err(err) => {
                    self.last_error = Some(err.clone());
                    return Err(err);
                }
            },
        };

        self.last_error = None;
        self.runtime = Some(runtime);
        Ok(())
    }

    pub fn deactivate(&mut self) {
        self.runtime = None;
        self.last_error = None;
    }

    pub fn embed_query(&self, query: &str) -> Option<Result<Vec<Vec<f32>>, String>> {
        let prepared = prepare_query_text(self.current_model_id(), query);
        let refs = [prepared.as_ref()];
        self.runtime
            .as_ref()
            .map(|runtime| runtime.embed_batch(&refs))
    }

    pub fn embed_documents(&self, texts: &[&str]) -> Option<Result<Vec<Vec<f32>>, String>> {
        let prepared: Vec<Cow<'_, str>> = texts
            .iter()
            .map(|text| prepare_document_text(self.current_model_id(), text))
            .collect();
        let refs: Vec<&str> = prepared.iter().map(|text| text.as_ref()).collect();
        self.runtime
            .as_ref()
            .map(|runtime| runtime.embed_batch(&refs))
    }

    fn current_model_id(&self) -> &str {
        if self.config.embedding_mode == "remote" {
            self.config.remote_model.trim()
        } else {
            self.config.local_model.trim()
        }
    }

    pub fn backend_signature(&self) -> EmbeddingBackendSignature {
        if let Some(runtime) = &self.runtime {
            return runtime.backend_signature();
        }

        let (runtime_name, model_id, model_revision, device_route, dimension) =
            if self.config.embedding_mode == "remote" {
                (
                    "openai_compatible_remote".to_string(),
                    self.config.remote_model.clone(),
                    self.config.remote_endpoint.clone(),
                    "remote".to_string(),
                    self.config.remote_dimensions as usize,
                )
            } else {
                let requested_model = self.config.local_model.trim();
                let fallback_dimension = model_dimension_for_id(requested_model);
                let fallback_revision = fastembed_model_for_id(requested_model)
                    .and_then(|model| fastembed_model_code(&model))
                    .unwrap_or_else(|| "manual".to_string());
                (
                    self.config.local_runtime.clone(),
                    self.config.local_model.clone(),
                    fallback_revision,
                    configured_local_device_route(&self.config.device_policy),
                    fallback_dimension,
                )
            };

        EmbeddingBackendSignature {
            runtime_name,
            model_id,
            model_revision,
            device_route,
            dimension,
            normalize: true,
        }
    }

    pub fn backend_signature_json(&self) -> String {
        serde_json::to_string(&self.backend_signature()).unwrap_or_default()
    }

    pub fn gpu_memory_bytes(&self) -> Option<u64> {
        self.runtime
            .as_ref()
            .and_then(|runtime| runtime.gpu_memory_bytes())
    }

    pub fn gpu_dedicated_memory_bytes(&self) -> Option<u64> {
        if self.config.embedding_mode == "remote" {
            return None;
        }
        preferred_gpu_dedicated_vram_bytes()
    }

    pub fn device_name(&self) -> Option<String> {
        if let Some(runtime) = &self.runtime {
            return runtime.device_name();
        }

        if self.config.embedding_mode == "remote" {
            return None;
        }

        configured_local_device_name(&self.config.device_policy)
    }
}

fn embedding_runtime_restart_required(current: &EmbeddingConfig, next: &EmbeddingConfig) -> bool {
    current.enabled != next.enabled
        || current.embedding_mode != next.embedding_mode
        || current.device_policy != next.device_policy
        || current.local_runtime != next.local_runtime
        || current.local_model != next.local_model
        || current.local_model_path != next.local_model_path
        || current.remote_endpoint != next.remote_endpoint
        || current.remote_api_key != next.remote_api_key
        || current.remote_model != next.remote_model
        || current.remote_dimensions != next.remote_dimensions
        || current.remote_max_batch != next.remote_max_batch
}

enum LocalEmbeddingEngine {
    Fastembed(Mutex<TextEmbedding>),
    #[cfg(windows)]
    Ort(Mutex<DirectMlOrtSession>),
}

struct LocalEmbeddingRuntime {
    engine: LocalEmbeddingEngine,
    batch_size: usize,
    signature: EmbeddingBackendSignature,
    device_name: String,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalOrtExecutionRoute {
    Cpu,
    DirectMl,
}

#[cfg(windows)]
#[derive(Debug)]
struct ModelConfigHints {
    num_attention_heads: Option<usize>,
    num_key_value_heads: Option<usize>,
    head_dim: Option<usize>,
}

#[cfg(windows)]
#[derive(Debug)]
struct DirectMlRuntimeAssets {
    model_file_path: PathBuf,
    onnx_file: Vec<u8>,
    external_initializers: Vec<(String, Vec<u8>)>,
    tokenizer_files: TokenizerFiles,
    config_hints: ModelConfigHints,
    pooling: Option<ManualPoolingMode>,
    output_key: Option<fastembed::OutputKey>,
    quantization: QuantizationMode,
}

#[cfg(windows)]
#[derive(Debug, Clone)]
struct DirectMlInputSchema {
    input_ids_name: String,
    attention_mask_name: Option<String>,
    token_type_ids_name: Option<String>,
    position_ids_name: Option<String>,
    past_key_values: Vec<DirectMlPastKeyValueInput>,
}

#[cfg(windows)]
#[derive(Debug, Clone)]
struct DirectMlPastKeyValueInput {
    layer_index: usize,
    key_name: String,
    value_name: String,
    element_type: TensorElementType,
    rank: usize,
    num_heads: usize,
    head_dim: usize,
}

#[cfg(windows)]
#[derive(Debug)]
struct DirectMlOrtSession {
    tokenizer: Tokenizer,
    session: Session,
    route: LocalOrtExecutionRoute,
    device_id: Option<i32>,
    device_name: String,
    input_schema: DirectMlInputSchema,
    pooling: Option<ManualPoolingMode>,
    output_key: Option<fastembed::OutputKey>,
    quantization: QuantizationMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalModelRoute {
    Preset,
    Manual,
}

impl LocalModelRoute {
    fn as_str(self) -> &'static str {
        match self {
            Self::Preset => "preset",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone)]
struct LocalModelSelection {
    requested_model: String,
    route: LocalModelRoute,
    known_model: Option<FastembedModel>,
    manual_model_dir: PathBuf,
}

fn manual_pooling_to_fastembed(pooling: ManualPoolingMode) -> Option<Pooling> {
    match pooling {
        ManualPoolingMode::Cls => Some(Pooling::Cls),
        ManualPoolingMode::Mean => Some(Pooling::Mean),
        ManualPoolingMode::LastToken => None,
    }
}

#[cfg(windows)]
fn manual_pooling_from_fastembed(pooling: Pooling) -> ManualPoolingMode {
    match pooling {
        Pooling::Cls => ManualPoolingMode::Cls,
        Pooling::Mean => ManualPoolingMode::Mean,
    }
}

#[cfg(windows)]
fn parse_model_config_hints(config_bytes: &[u8]) -> ModelConfigHints {
    let Ok(config) = serde_json::from_slice::<serde_json::Value>(config_bytes) else {
        return ModelConfigHints {
            num_attention_heads: None,
            num_key_value_heads: None,
            head_dim: None,
        };
    };

    let num_attention_heads = config["num_attention_heads"]
        .as_u64()
        .map(|value| value as usize);
    let num_key_value_heads = config["num_key_value_heads"]
        .as_u64()
        .map(|value| value as usize);
    let hidden_size = config["hidden_size"].as_u64().map(|value| value as usize);
    let head_dim = config["head_dim"]
        .as_u64()
        .map(|value| value as usize)
        .or_else(|| match (hidden_size, num_attention_heads) {
            (Some(hidden_size), Some(num_attention_heads)) if num_attention_heads > 0 => {
                Some(hidden_size / num_attention_heads)
            }
            _ => None,
        });

    ModelConfigHints {
        num_attention_heads,
        num_key_value_heads,
        head_dim,
    }
}

#[cfg(windows)]
fn detect_directml_input_schema(
    inputs: &[ort::value::Outlet],
    config_hints: &ModelConfigHints,
) -> Result<DirectMlInputSchema, String> {
    let input_ids_name = find_named_tensor_input(inputs, &["input_ids"])
        .or_else(|| {
            inputs
                .iter()
                .find(|input| {
                    matches!(
                        input.dtype(),
                        ort::value::ValueType::Tensor {
                            ty: TensorElementType::Int64,
                            shape,
                            ..
                        } if shape.len() == 2
                            && !matches_special_embedding_input_name(input.name())
                    )
                })
                .map(|input| input.name().to_string())
        })
        .ok_or_else(|| {
            format!(
                "Could not detect the primary token id input from model inputs: {:?}",
                inputs.iter().map(|input| input.name()).collect::<Vec<_>>()
            )
        })?;
    let attention_mask_name = find_named_tensor_input(inputs, &["attention_mask"]).or_else(|| {
        inputs
            .iter()
            .find(|input| {
                input.name().to_ascii_lowercase().contains("mask")
                    && matches!(
                        input.dtype(),
                        ort::value::ValueType::Tensor {
                            ty: TensorElementType::Int64,
                            shape,
                            ..
                        } if shape.len() == 2
                    )
            })
            .map(|input| input.name().to_string())
    });
    let token_type_ids_name = find_named_tensor_input(inputs, &["token_type_ids", "segment_ids"]);
    let position_ids_name = find_named_tensor_input(inputs, &["position_ids"]);

    let mut pending = BTreeMap::<usize, PendingPastKeyValueInput>::new();
    for input in inputs {
        let Some((layer_index, cache_kind)) = parse_past_key_value_input_name(input.name()) else {
            continue;
        };
        let Some((element_type, rank, num_heads, head_dim)) =
            parse_past_key_value_shape(input, config_hints)
        else {
            continue;
        };
        let entry = pending
            .entry(layer_index)
            .or_insert_with(|| PendingPastKeyValueInput {
                key_name: None,
                value_name: None,
                element_type,
                rank,
                num_heads,
                head_dim,
            });
        entry.element_type = element_type;
        entry.rank = rank;
        entry.num_heads = num_heads;
        entry.head_dim = head_dim;
        match cache_kind {
            PastKeyValueInputKind::Key => entry.key_name = Some(input.name().to_string()),
            PastKeyValueInputKind::Value => entry.value_name = Some(input.name().to_string()),
        }
    }

    let past_key_values = pending
        .into_iter()
        .filter_map(|(layer_index, entry)| {
            Some(DirectMlPastKeyValueInput {
                layer_index,
                key_name: entry.key_name?,
                value_name: entry.value_name?,
                element_type: entry.element_type,
                rank: entry.rank,
                num_heads: entry.num_heads,
                head_dim: entry.head_dim,
            })
        })
        .collect();

    Ok(DirectMlInputSchema {
        input_ids_name,
        attention_mask_name,
        token_type_ids_name,
        position_ids_name,
        past_key_values,
    })
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
enum PastKeyValueInputKind {
    Key,
    Value,
}

#[cfg(windows)]
#[derive(Debug)]
struct PendingPastKeyValueInput {
    key_name: Option<String>,
    value_name: Option<String>,
    element_type: TensorElementType,
    rank: usize,
    num_heads: usize,
    head_dim: usize,
}

#[cfg(windows)]
fn matches_special_embedding_input_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    normalized.contains("mask")
        || normalized.contains("position")
        || normalized.contains("token_type")
        || normalized.starts_with("past_key_values.")
}

#[cfg(windows)]
fn find_named_tensor_input(inputs: &[ort::value::Outlet], candidates: &[&str]) -> Option<String> {
    inputs
        .iter()
        .find(|input| {
            candidates
                .iter()
                .any(|candidate| input.name().eq_ignore_ascii_case(candidate))
        })
        .map(|input| input.name().to_string())
}

#[cfg(windows)]
fn parse_past_key_value_input_name(name: &str) -> Option<(usize, PastKeyValueInputKind)> {
    let normalized = name.trim();
    let suffix = if normalized.ends_with(".key") {
        PastKeyValueInputKind::Key
    } else if normalized.ends_with(".value") {
        PastKeyValueInputKind::Value
    } else {
        return None;
    };
    let prefix = normalized
        .trim_end_matches(".key")
        .trim_end_matches(".value");
    let layer_index = prefix
        .strip_prefix("past_key_values.")?
        .parse::<usize>()
        .ok()?;
    Some((layer_index, suffix))
}

#[cfg(windows)]
fn parse_past_key_value_shape(
    input: &ort::value::Outlet,
    config_hints: &ModelConfigHints,
) -> Option<(TensorElementType, usize, usize, usize)> {
    let ort::value::ValueType::Tensor { ty, shape, .. } = input.dtype() else {
        return None;
    };
    let rank = shape.len();
    if rank < 3 {
        return None;
    }

    let num_heads = shape
        .get(1)
        .copied()
        .filter(|value| *value > 0)
        .map(|value| value as usize)
        .or(config_hints.num_key_value_heads)
        .or(config_hints.num_attention_heads)
        .unwrap_or(1);
    let head_dim = shape
        .last()
        .copied()
        .filter(|value| *value > 0)
        .map(|value| value as usize)
        .or(config_hints.head_dim)
        .unwrap_or(1);

    Some((*ty, rank, num_heads, head_dim))
}

#[cfg(windows)]
fn build_position_ids_array(attention_mask_array: &Array2<i64>) -> Array2<i64> {
    let (batch_size, sequence_length) = attention_mask_array.dim();
    let mut position_ids = Array2::<i64>::zeros((batch_size, sequence_length));

    for batch_index in 0..batch_size {
        let mut next_position = 0_i64;
        for token_index in 0..sequence_length {
            if attention_mask_array[(batch_index, token_index)] != 0 {
                position_ids[(batch_index, token_index)] = next_position;
                next_position += 1;
            }
        }
    }

    position_ids
}

#[cfg(windows)]
fn build_zero_past_key_value_tensor(
    allocator: &ort::memory::Allocator,
    input: &DirectMlPastKeyValueInput,
    batch_size: usize,
) -> Result<ort::value::DynTensor, String> {
    let shape = match input.rank {
        0 => vec![],
        1 => vec![batch_size],
        2 => vec![batch_size, 0],
        3 => vec![batch_size, 0, input.head_dim],
        _ => vec![batch_size, input.num_heads, 0, input.head_dim],
    };
    ort::value::DynTensor::new(allocator, input.element_type, shape).map_err(|error| {
        format!(
            "Failed to build KV cache tensor for layer {} ('{}'): {error}",
            input.layer_index, input.key_name
        )
    })
}

fn select_local_model_source(
    config: &EmbeddingConfig,
    model_dir: &Path,
) -> Result<LocalModelSelection, String> {
    let requested_model = config.local_model.trim().to_string();
    if requested_model.is_empty() && config.local_model_path.trim().is_empty() {
        return Err("Local embedding model is not configured".to_string());
    }

    let manual_model_dir = configured_local_model_dir(config, model_dir);
    let known_model = fastembed_model_for_id(&requested_model);
    let supported_preset = supported_embedding_preset(&requested_model);

    if config.local_model_path.trim().is_empty()
        && (known_model.is_some() || supported_preset.is_some())
    {
        return Ok(LocalModelSelection {
            requested_model,
            route: LocalModelRoute::Preset,
            known_model,
            manual_model_dir,
        });
    }

    if manual_model_files_ready(&manual_model_dir) {
        return Ok(LocalModelSelection {
            requested_model,
            route: LocalModelRoute::Manual,
            known_model,
            manual_model_dir,
        });
    }

    if !config.local_model_path.trim().is_empty() {
        return Err(format!(
            "Local model directory '{}' is missing required ONNX or tokenizer files",
            manual_model_dir.display()
        ));
    }

    if known_model.is_some() {
        return Ok(LocalModelSelection {
            requested_model,
            route: LocalModelRoute::Preset,
            known_model,
            manual_model_dir,
        });
    }

    Err(local_model_not_downloaded_message(
        &requested_model,
        &manual_model_dir,
    ))
}

fn local_model_not_downloaded_message(model_id: &str, expected_dir: &Path) -> String {
    format!(
        "Local embedding model '{}' has not been downloaded. Download the model manually before activating semantic search. Expected local files in: {}",
        model_id,
        expected_dir.display()
    )
}

impl LocalEmbeddingRuntime {
    fn ensure_selected_local_model_ready(
        selection: &LocalModelSelection,
        model_dir: &Path,
    ) -> Result<(), String> {
        if let Some(model) = selection.known_model.clone() {
            if fastembed_model_cached(&effective_fastembed_cache_dir(model_dir), &model) {
                return Ok(());
            }
            return Err(local_model_not_downloaded_message(
                &selection.requested_model,
                &selection.manual_model_dir,
            ));
        }

        if manual_model_files_ready(&selection.manual_model_dir) {
            return Ok(());
        }

        Err(local_model_not_downloaded_message(
            &selection.requested_model,
            &selection.manual_model_dir,
        ))
    }

    fn new_with_requested_backend_only(
        config: &EmbeddingConfig,
        model_dir: &Path,
    ) -> Result<Self, String> {
        preload_ort_helpers();

        let selection = select_local_model_source(config, model_dir)?;
        match selection.route {
            LocalModelRoute::Preset => {
                Self::ensure_selected_local_model_ready(&selection, model_dir)?;
                if let Some(model) = selection.known_model.clone() {
                    return Self::build_fastembed_preset_runtime(
                        &selection.requested_model,
                        model_dir,
                        model,
                        &config.device_policy,
                    );
                }
                Self::new_from_manual_files(
                    &selection.requested_model,
                    &selection.manual_model_dir,
                    None,
                    &config.device_policy,
                )
            }
            LocalModelRoute::Manual => Self::new_from_manual_files(
                &selection.requested_model,
                &selection.manual_model_dir,
                selection.known_model.as_ref(),
                &config.device_policy,
            ),
        }
    }

    fn new_with_progress<F>(
        config: &EmbeddingConfig,
        model_dir: &Path,
        on_progress: &mut F,
    ) -> Result<Self, String>
    where
        F: FnMut(EmbeddingActivationProgress),
    {
        let runtime_name = config.local_runtime.trim();
        if !runtime_name.is_empty() && !runtime_name.eq_ignore_ascii_case(LOCAL_RUNTIME_FASTEMBED) {
            return Err(format!(
                "Unsupported local embedding runtime '{}'. Supported runtime: {}.",
                config.local_runtime, LOCAL_RUNTIME_FASTEMBED
            ));
        }

        preload_ort_helpers();

        let selection = select_local_model_source(config, model_dir)?;
        match selection.route {
            LocalModelRoute::Preset => {
                Self::ensure_selected_local_model_ready(&selection, model_dir)?;
                on_progress(EmbeddingActivationProgress::Stage {
                    stage: "initializing_runtime",
                    detail: Some(format!(
                        "Initializing local model '{}'",
                        selection.requested_model
                    )),
                });
                if let Some(model) = selection.known_model.clone() {
                    return Self::new_from_fastembed_model(
                        &selection.requested_model,
                        model_dir,
                        model,
                        &config.device_policy,
                    );
                }
                Self::new_from_manual_files(
                    &selection.requested_model,
                    &selection.manual_model_dir,
                    None,
                    &config.device_policy,
                )
            }
            LocalModelRoute::Manual => {
                on_progress(EmbeddingActivationProgress::Stage {
                    stage: "initializing_runtime",
                    detail: Some(format!(
                        "Loading manual model files for '{}'",
                        selection.requested_model
                    )),
                });
                Self::new_from_manual_files(
                    &selection.requested_model,
                    &selection.manual_model_dir,
                    selection.known_model.as_ref(),
                    &config.device_policy,
                )
            }
        }
    }

    fn new_from_fastembed_model(
        requested_model: &str,
        model_dir: &Path,
        model: FastembedModel,
        device_policy: &str,
    ) -> Result<Self, String> {
        #[cfg(windows)]
        if normalize_device_policy(device_policy) == DEVICE_POLICY_GPU_DIRECTML {
            match Self::build_fastembed_preset_runtime(
                requested_model,
                model_dir,
                model.clone(),
                device_policy,
            ) {
                Ok(runtime) => return Ok(runtime),
                Err(error) => {
                    tracing::warn!(
                        log_module = "knowledge_index",
                        requested_model = %requested_model,
                        device_policy = %device_policy,
                        fallback = "cpu_fastembed",
                        "{error}"
                    );
                    return Self::build_fastembed_preset_runtime(
                        requested_model,
                        model_dir,
                        model,
                        DEVICE_POLICY_CPU_FASTEMBED,
                    );
                }
            }
        }

        Self::build_fastembed_preset_runtime(requested_model, model_dir, model, device_policy)
    }

    fn build_fastembed_preset_runtime(
        requested_model: &str,
        model_dir: &Path,
        model: FastembedModel,
        device_policy: &str,
    ) -> Result<Self, String> {
        preload_ort_helpers();

        let cache_dir = effective_fastembed_cache_dir(model_dir);
        let (mut embedding, device) = initialize_local_embedding_with_device_policy(
            requested_model,
            model_dir,
            device_policy,
            |execution_providers| {
                let options = TextInitOptions::new(model.clone())
                    .with_cache_dir(cache_dir.clone())
                    .with_execution_providers(execution_providers)
                    .with_show_download_progress(false);

                TextEmbedding::try_new(options).map_err(|e| {
                    format!(
                        "Failed to initialize local embedding model '{}': {}",
                        requested_model,
                        normalize_runtime_error_message_with_debug(
                            &e.to_string(),
                            Some(&format!("{e:#?}")),
                            "Unknown fastembed initialization error"
                        )
                    )
                })
            },
        )?;

        let dimension = fastembed_model_dimension(&model)
            .or_else(|| probe_embedding_dimension(&mut embedding).ok())
            .unwrap_or(0);
        let model_revision = fastembed_model_code(&model).unwrap_or_else(|| "unknown".to_string());

        Ok(Self {
            engine: LocalEmbeddingEngine::Fastembed(Mutex::new(embedding)),
            batch_size: LOCAL_EMBED_BATCH_SIZE,
            signature: build_local_backend_signature(
                requested_model,
                model_revision,
                device.route,
                dimension,
            ),
            device_name: device.device_name,
        })
    }

    fn new_from_manual_files(
        requested_model: &str,
        manual_model_dir: &Path,
        known_model: Option<&FastembedModel>,
        device_policy: &str,
    ) -> Result<Self, String> {
        #[cfg(windows)]
        if normalize_device_policy(device_policy) == DEVICE_POLICY_GPU_DIRECTML {
            match Self::build_directml_manual_runtime(
                requested_model,
                manual_model_dir,
                known_model,
            ) {
                Ok(runtime) => return Ok(runtime),
                Err(error) => {
                    tracing::warn!(
                        log_module = "knowledge_index",
                        requested_model = %requested_model,
                        device_policy = %device_policy,
                        fallback = "cpu_fastembed",
                        "{error}"
                    );
                    return Self::build_cpu_manual_runtime(
                        requested_model,
                        manual_model_dir,
                        known_model,
                    );
                }
            }
        }

        #[cfg(windows)]
        if normalize_device_policy(device_policy) == DEVICE_POLICY_CPU_FASTEMBED {
            return Self::build_cpu_manual_runtime(requested_model, manual_model_dir, known_model);
        }

        Self::build_fastembed_manual_runtime(
            requested_model,
            manual_model_dir,
            known_model,
            device_policy,
        )
    }

    fn build_fastembed_manual_runtime(
        requested_model: &str,
        manual_model_dir: &Path,
        known_model: Option<&FastembedModel>,
        device_policy: &str,
    ) -> Result<Self, String> {
        preload_ort_helpers();

        let model_file = find_manual_model_file(manual_model_dir).ok_or_else(|| {
            format!(
                "No ONNX model file was found in {}",
                manual_model_dir.display()
            )
        })?;

        let tokenizer_files = load_manual_tokenizer_files(manual_model_dir)?;
        let onnx_file = std::fs::read(&model_file).map_err(|e| {
            format!(
                "Failed to read ONNX model '{}': {}",
                model_file.display(),
                e
            )
        })?;

        let mut user_model = UserDefinedEmbeddingModel::new(onnx_file, tokenizer_files);
        if let Some(model) = known_model {
            if let Some(pooling) = TextEmbedding::get_default_pooling_method(model) {
                user_model = user_model.with_pooling(pooling);
            }
            let quantization = TextEmbedding::get_quantization_mode(model);
            if quantization != QuantizationMode::None {
                user_model = user_model.with_quantization(quantization);
            }
            if let Ok(info) = TextEmbedding::get_model_info(model) {
                user_model.output_key = info.output_key.clone();
            }
        } else if let Some(pooling) = load_manual_pooling(manual_model_dir, requested_model)? {
            let fastembed_pooling = manual_pooling_to_fastembed(pooling).ok_or_else(|| {
                format!(
                    "Manual local embedding model '{}' requires last-token pooling, which the current fastembed manual runtime does not support on this platform",
                    manual_model_dir.display()
                )
            })?;
            user_model = user_model.with_pooling(fastembed_pooling);
        }

        for (file_name, buffer) in collect_manual_external_initializers(&model_file)? {
            user_model = user_model.with_external_initializer(file_name, buffer);
        }

        let (mut embedding, device) = initialize_local_embedding_with_device_policy(
            requested_model,
            manual_model_dir,
            device_policy,
            |execution_providers| {
                TextEmbedding::try_new_from_user_defined(
                    user_model.clone(),
                    InitOptionsUserDefined::new().with_execution_providers(execution_providers),
                )
                .map_err(|e| {
                    format!(
                        "Failed to initialize manual local embedding model '{}': {}",
                        manual_model_dir.display(),
                        normalize_runtime_error_message_with_debug(
                            &e.to_string(),
                            Some(&format!("{e:#?}")),
                            "Unknown fastembed initialization error"
                        )
                    )
                })
            },
        )?;

        let dimension = known_model
            .and_then(fastembed_model_dimension)
            .or_else(|| probe_embedding_dimension(&mut embedding).ok())
            .unwrap_or(0);
        let model_revision = known_model
            .and_then(fastembed_model_code)
            .unwrap_or_else(|| "manual".to_string());

        Ok(Self {
            engine: LocalEmbeddingEngine::Fastembed(Mutex::new(embedding)),
            batch_size: LOCAL_EMBED_BATCH_SIZE,
            signature: build_local_backend_signature(
                requested_model,
                model_revision,
                device.route,
                dimension,
            ),
            device_name: device.device_name,
        })
    }

    #[cfg(windows)]
    fn build_directml_preset_runtime(
        requested_model: &str,
        model_dir: &Path,
        model: &FastembedModel,
    ) -> Result<Self, String> {
        let assets = resolve_directml_assets_for_fastembed_model(model_dir, model)?;
        let model_revision = fastembed_model_code(model).unwrap_or_else(|| "unknown".to_string());
        let mut ort_session = DirectMlOrtSession::new_directml(assets)?;
        let dimension = fastembed_model_dimension(model)
            .or_else(|| ort_session.probe_dimension().ok())
            .unwrap_or(0);
        let device_name = ort_session.device_name.clone();

        Ok(Self {
            engine: LocalEmbeddingEngine::Ort(Mutex::new(ort_session)),
            batch_size: LOCAL_EMBED_BATCH_SIZE,
            signature: build_local_backend_signature(
                requested_model,
                model_revision,
                "directml".to_string(),
                dimension,
            ),
            device_name,
        })
    }

    #[cfg(windows)]
    fn build_directml_manual_runtime(
        requested_model: &str,
        manual_model_dir: &Path,
        known_model: Option<&FastembedModel>,
    ) -> Result<Self, String> {
        let model_revision = known_model
            .and_then(fastembed_model_code)
            .unwrap_or_else(|| "manual".to_string());
        let assets = resolve_directml_assets_for_manual_model(
            manual_model_dir,
            requested_model,
            known_model,
        )?;
        let mut ort_session = DirectMlOrtSession::new_directml(assets)?;
        let dimension = known_model
            .and_then(fastembed_model_dimension)
            .or_else(|| ort_session.probe_dimension().ok())
            .unwrap_or(0);
        let device_name = ort_session.device_name.clone();

        Ok(Self {
            engine: LocalEmbeddingEngine::Ort(Mutex::new(ort_session)),
            batch_size: LOCAL_EMBED_BATCH_SIZE,
            signature: build_local_backend_signature(
                requested_model,
                model_revision,
                "directml".to_string(),
                dimension,
            ),
            device_name,
        })
    }

    #[cfg(windows)]
    fn build_cpu_manual_runtime(
        requested_model: &str,
        manual_model_dir: &Path,
        known_model: Option<&FastembedModel>,
    ) -> Result<Self, String> {
        let model_revision = known_model
            .and_then(fastembed_model_code)
            .unwrap_or_else(|| "manual".to_string());
        let assets = resolve_directml_assets_for_manual_model(
            manual_model_dir,
            requested_model,
            known_model,
        )?;
        let mut ort_session = DirectMlOrtSession::new_cpu(assets)?;
        let dimension = known_model
            .and_then(fastembed_model_dimension)
            .or_else(|| ort_session.probe_dimension().ok())
            .unwrap_or(0);
        let device_name = ort_session.device_name.clone();

        Ok(Self {
            engine: LocalEmbeddingEngine::Ort(Mutex::new(ort_session)),
            batch_size: LOCAL_EMBED_BATCH_SIZE,
            signature: build_local_backend_signature(
                requested_model,
                model_revision,
                "cpu".to_string(),
                dimension,
            ),
            device_name,
        })
    }
}

impl EmbeddingRuntime for LocalEmbeddingRuntime {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        match &self.engine {
            LocalEmbeddingEngine::Fastembed(embedding) => {
                let mut embedding = embedding
                    .lock()
                    .map_err(|_| "Local embedding runtime is unavailable".to_string())?;
                embedding
                    .embed(texts, Some(self.batch_size.min(texts.len().max(1))))
                    .map_err(|e| format!("Local embedding failed: {}", e))
            }
            #[cfg(windows)]
            LocalEmbeddingEngine::Ort(session) => {
                let mut session = session
                    .lock()
                    .map_err(|_| "Local ONNX embedding runtime is unavailable".to_string())?;
                session.embed_batch(texts, self.batch_size)
            }
        }
    }

    fn backend_signature(&self) -> EmbeddingBackendSignature {
        self.signature.clone()
    }

    fn device_name(&self) -> Option<String> {
        let trimmed = self.device_name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn gpu_memory_bytes(&self) -> Option<u64> {
        match &self.engine {
            #[cfg(windows)]
            LocalEmbeddingEngine::Ort(session) => session
                .lock()
                .ok()
                .and_then(|session| {
                    if session.route == LocalOrtExecutionRoute::DirectMl {
                        session
                            .device_id
                            .and_then(query_gpu_memory_bytes_for_adapter_index)
                    } else {
                        None
                    }
                })
                .or_else(query_preferred_gpu_memory_bytes),
            #[cfg(windows)]
            _ if self.signature.device_route == "directml"
                || self.signature.device_route == "cuda" =>
            {
                query_preferred_gpu_memory_bytes()
            }
            _ => None,
        }
    }
}

pub fn run_embedding_runtime_self_test(
    config: &EmbeddingConfig,
    model_storage_dir: &Path,
) -> Result<EmbeddingRuntimeTestResult, String> {
    let config = sanitize_embedding_config(config.clone());
    let test_text = "semantic retrieval self test";

    if config.embedding_mode == "remote" {
        let case = run_remote_embedding_runtime_self_test_case(&config, test_text);
        let cases = vec![case.clone()];
        let summary = build_embedding_runtime_self_test_summary(&case, &cases);
        return Ok(EmbeddingRuntimeTestResult {
            passed: case.outcome == "passed",
            summary,
            backend: case.backend.clone(),
            model_id: case.model_id.clone(),
            dimension: case.dimension,
            vector_count: case.vector_count,
            latency_ms: case.latency_ms,
            cases,
            diagnostics: None,
        });
    }

    let model_root = managed_model_root(model_storage_dir);
    let current_selection = select_local_model_source(&config, &model_root)?;
    let current_case = run_current_local_embedding_runtime_self_test_case(
        &config,
        &model_root,
        current_selection.route,
        test_text,
    );
    let mut cases = vec![current_case.clone()];
    #[cfg(windows)]
    {
        let current_case_id = current_case.case_id.clone();
        cases.extend(run_local_embedding_runtime_matrix_cases(
            &config,
            &model_root,
            test_text,
            &current_case_id,
        ));
    }

    let diagnostics = collect_embedding_runtime_test_diagnostics();
    let summary = build_embedding_runtime_self_test_summary(&current_case, &cases);
    tracing::info!(
        log_module = "knowledge_index",
        summary = %summary,
        cases = %serde_json::to_string(&cases).unwrap_or_default(),
        diagnostics = %serde_json::to_string(&diagnostics).unwrap_or_default(),
        "embedding runtime self-test completed"
    );

    Ok(EmbeddingRuntimeTestResult {
        passed: current_case.outcome == "passed",
        summary,
        backend: current_case.backend.clone(),
        model_id: current_case.model_id.clone(),
        dimension: current_case.dimension,
        vector_count: current_case.vector_count,
        latency_ms: current_case.latency_ms,
        cases,
        diagnostics,
    })
}

fn run_remote_embedding_runtime_self_test_case(
    config: &EmbeddingConfig,
    test_text: &str,
) -> EmbeddingRuntimeTestCaseResult {
    let started = Instant::now();
    match RemoteEmbeddingRuntime::new(config) {
        Ok(runtime) => {
            let signature = runtime.backend_signature();
            match runtime.embed_batch(&[test_text]) {
                Ok(vectors) => build_passed_self_test_case(
                    "remote".to_string(),
                    "remote".to_string(),
                    "remote".to_string(),
                    signature,
                    vectors,
                    started.elapsed().as_millis() as u64,
                ),
                Err(error) => build_failed_self_test_case(
                    "remote".to_string(),
                    "remote".to_string(),
                    "remote".to_string(),
                    signature.runtime_name.clone(),
                    signature.model_id.clone(),
                    started.elapsed().as_millis() as u64,
                    error,
                ),
            }
        }
        Err(error) => build_failed_self_test_case(
            "remote".to_string(),
            "remote".to_string(),
            "remote".to_string(),
            "openai_compatible_remote".to_string(),
            config.remote_model.trim().to_string(),
            started.elapsed().as_millis() as u64,
            error,
        ),
    }
}

fn run_current_local_embedding_runtime_self_test_case(
    config: &EmbeddingConfig,
    model_root: &Path,
    route: LocalModelRoute,
    test_text: &str,
) -> EmbeddingRuntimeTestCaseResult {
    let provider = self_test_provider_label_from_policy(&config.device_policy).to_string();
    let case_id = format!("{}+{}", route.as_str(), provider);
    let requested_model = config.local_model.trim().to_string();
    let backend = fallback_self_test_backend_label(&provider);

    execute_local_self_test_case(
        case_id,
        route.as_str().to_string(),
        provider,
        requested_model,
        backend,
        || LocalEmbeddingRuntime::new_with_requested_backend_only(config, model_root),
        test_text,
    )
}

#[cfg(windows)]
fn run_local_embedding_runtime_matrix_cases(
    config: &EmbeddingConfig,
    model_root: &Path,
    test_text: &str,
    skip_case_id: &str,
) -> Vec<EmbeddingRuntimeTestCaseResult> {
    let requested_model = config.local_model.trim().to_string();
    let manual_model_dir = configured_local_model_dir(config, model_root);
    let known_model = fastembed_model_for_id(&requested_model);
    let manual_ready = manual_model_files_ready(&manual_model_dir);
    let missing_manual_files = required_local_model_files(&manual_model_dir);
    let mut cases = Vec::new();

    for (route, device_policy) in [
        (LocalModelRoute::Preset, DEVICE_POLICY_CPU_FASTEMBED),
        (LocalModelRoute::Preset, DEVICE_POLICY_GPU_DIRECTML),
        (LocalModelRoute::Manual, DEVICE_POLICY_CPU_FASTEMBED),
        (LocalModelRoute::Manual, DEVICE_POLICY_GPU_DIRECTML),
    ] {
        let provider = self_test_provider_label_from_policy(device_policy).to_string();
        let case_id = format!("{}+{}", route.as_str(), provider);
        if case_id == skip_case_id {
            continue;
        }

        let backend = fallback_self_test_backend_label(&provider);
        let case = match route {
            LocalModelRoute::Preset => {
                if let Some(model) = known_model.clone() {
                    if fastembed_model_cached(&effective_fastembed_cache_dir(model_root), &model) {
                        execute_local_self_test_case(
                            case_id,
                            route.as_str().to_string(),
                            provider,
                            requested_model.clone(),
                            backend,
                            || {
                                LocalEmbeddingRuntime::build_fastembed_preset_runtime(
                                    &requested_model,
                                    model_root,
                                    model.clone(),
                                    device_policy,
                                )
                            },
                            test_text,
                        )
                    } else {
                        skipped_self_test_case(
                            case_id,
                            route.as_str().to_string(),
                            provider,
                            backend,
                            requested_model.clone(),
                            local_model_not_downloaded_message(&requested_model, &manual_model_dir),
                        )
                    }
                } else {
                    skipped_self_test_case(
                        case_id,
                        route.as_str().to_string(),
                        provider,
                        backend,
                        requested_model.clone(),
                        format!(
                            "Preset model '{}' is unavailable for automatic download",
                            requested_model
                        ),
                    )
                }
            }
            LocalModelRoute::Manual => {
                if manual_ready {
                    let known_model = known_model.clone();
                    execute_local_self_test_case(
                        case_id,
                        route.as_str().to_string(),
                        provider,
                        requested_model.clone(),
                        backend,
                        || {
                            LocalEmbeddingRuntime::build_fastembed_manual_runtime(
                                &requested_model,
                                &manual_model_dir,
                                known_model.as_ref(),
                                device_policy,
                            )
                        },
                        test_text,
                    )
                } else {
                    let reason = if missing_manual_files.is_empty() {
                        format!(
                            "Manual model files are unavailable in {}",
                            manual_model_dir.display()
                        )
                    } else {
                        format!(
                            "Manual model files are unavailable in {} (missing: {})",
                            manual_model_dir.display(),
                            missing_manual_files.join(", ")
                        )
                    };
                    skipped_self_test_case(
                        case_id,
                        route.as_str().to_string(),
                        provider,
                        backend,
                        requested_model.clone(),
                        reason,
                    )
                }
            }
        };

        cases.push(case);
    }

    cases
}

fn execute_local_self_test_case<F>(
    case_id: String,
    route: String,
    provider: String,
    requested_model: String,
    fallback_backend: String,
    mut build: F,
    test_text: &str,
) -> EmbeddingRuntimeTestCaseResult
where
    F: FnMut() -> Result<LocalEmbeddingRuntime, String>,
{
    let started = Instant::now();
    match build() {
        Ok(runtime) => {
            let signature = runtime.backend_signature();
            match runtime.embed_batch(&[test_text]) {
                Ok(vectors) => build_passed_self_test_case(
                    case_id,
                    route,
                    provider,
                    signature,
                    vectors,
                    started.elapsed().as_millis() as u64,
                ),
                Err(error) => build_failed_self_test_case(
                    case_id,
                    route,
                    provider,
                    self_test_backend_label(&signature),
                    non_empty_model_id(&signature.model_id, &requested_model),
                    started.elapsed().as_millis() as u64,
                    error,
                ),
            }
        }
        Err(error) => build_failed_self_test_case(
            case_id,
            route,
            provider,
            fallback_backend,
            requested_model,
            started.elapsed().as_millis() as u64,
            error,
        ),
    }
}

fn build_passed_self_test_case(
    case_id: String,
    route: String,
    provider: String,
    signature: EmbeddingBackendSignature,
    vectors: Vec<Vec<f32>>,
    latency_ms: u64,
) -> EmbeddingRuntimeTestCaseResult {
    let dimension = vectors
        .first()
        .map(Vec::len)
        .filter(|dimension| *dimension > 0)
        .unwrap_or(signature.dimension);
    EmbeddingRuntimeTestCaseResult {
        case_id,
        route,
        provider,
        backend: self_test_backend_label(&signature),
        model_id: non_empty_model_id(&signature.model_id, "manual"),
        dimension,
        vector_count: vectors.len(),
        latency_ms,
        outcome: "passed".to_string(),
        error: None,
    }
}

fn build_failed_self_test_case(
    case_id: String,
    route: String,
    provider: String,
    backend: String,
    model_id: String,
    latency_ms: u64,
    error: String,
) -> EmbeddingRuntimeTestCaseResult {
    EmbeddingRuntimeTestCaseResult {
        case_id,
        route,
        provider,
        backend,
        model_id: non_empty_model_id(&model_id, "manual"),
        dimension: 0,
        vector_count: 0,
        latency_ms,
        outcome: "failed".to_string(),
        error: Some(error),
    }
}

fn skipped_self_test_case(
    case_id: String,
    route: String,
    provider: String,
    backend: String,
    model_id: String,
    error: String,
) -> EmbeddingRuntimeTestCaseResult {
    EmbeddingRuntimeTestCaseResult {
        case_id,
        route,
        provider,
        backend,
        model_id: non_empty_model_id(&model_id, "manual"),
        dimension: 0,
        vector_count: 0,
        latency_ms: 0,
        outcome: "skipped".to_string(),
        error: Some(error),
    }
}

fn build_embedding_runtime_self_test_summary(
    current_case: &EmbeddingRuntimeTestCaseResult,
    cases: &[EmbeddingRuntimeTestCaseResult],
) -> String {
    let base = match current_case.outcome.as_str() {
        "passed" => format!(
            "{} self-test passed: model={}, dim={}, vectors={}, latency={}ms",
            current_case.backend,
            non_empty_model_id(&current_case.model_id, "manual"),
            current_case.dimension,
            current_case.vector_count,
            current_case.latency_ms
        ),
        "skipped" => format!(
            "{} self-test skipped: {}",
            current_case.backend,
            current_case
                .error
                .as_deref()
                .unwrap_or("no compatible test route was available")
        ),
        _ => format!(
            "{} self-test failed: {}",
            current_case.backend,
            current_case
                .error
                .as_deref()
                .unwrap_or("runtime returned an unknown error")
        ),
    };

    if cases.len() <= 1 {
        return base;
    }

    format!(
        "{}. Matrix: passed=[{}], failed=[{}], skipped=[{}]",
        base,
        format_self_test_case_list(cases, "passed"),
        format_self_test_case_list(cases, "failed"),
        format_self_test_case_list(cases, "skipped"),
    )
}

fn format_self_test_case_list(cases: &[EmbeddingRuntimeTestCaseResult], outcome: &str) -> String {
    let names = cases
        .iter()
        .filter(|case| case.outcome == outcome)
        .map(|case| case.case_id.clone())
        .collect::<Vec<_>>();
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

fn self_test_backend_label(signature: &EmbeddingBackendSignature) -> String {
    if signature.device_route == "remote" {
        signature.runtime_name.clone()
    } else {
        format!("{} ({})", signature.runtime_name, signature.device_route)
    }
}

fn fallback_self_test_backend_label(provider: &str) -> String {
    format!(
        "{} ({})",
        LOCAL_RUNTIME_FASTEMBED,
        match provider {
            "gpu_directml" => "directml",
            "gpu_cuda" => "cuda",
            _ => "cpu",
        }
    )
}

fn self_test_provider_label_from_policy(device_policy: &str) -> &'static str {
    match normalize_device_policy(device_policy) {
        DEVICE_POLICY_GPU_DIRECTML => "gpu_directml",
        DEVICE_POLICY_GPU_CUDA => "gpu_cuda",
        _ => "cpu_fastembed",
    }
}

fn non_empty_model_id(model_id: &str, fallback: &str) -> String {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(windows)]
fn collect_embedding_runtime_test_diagnostics() -> Option<EmbeddingRuntimeTestDiagnostics> {
    Some(EmbeddingRuntimeTestDiagnostics {
        dlls: collect_runtime_helper_dlls(),
        adapters: collect_directml_adapter_diagnostics(),
    })
}

#[cfg(not(windows))]
fn collect_embedding_runtime_test_diagnostics() -> Option<EmbeddingRuntimeTestDiagnostics> {
    None
}

fn build_local_backend_signature(
    requested_model: &str,
    model_revision: String,
    device_route: String,
    dimension: usize,
) -> EmbeddingBackendSignature {
    EmbeddingBackendSignature {
        runtime_name: LOCAL_RUNTIME_FASTEMBED.to_string(),
        model_id: requested_model.to_string(),
        model_revision,
        device_route,
        dimension,
        normalize: true,
    }
}

#[cfg(windows)]
const DIRECTML_OUTPUT_PRECEDENCE: &[fastembed::OutputKey] = &[
    fastembed::OutputKey::OnlyOne,
    fastembed::OutputKey::ByName("text_embeds"),
    fastembed::OutputKey::ByName("sentence_embedding"),
    fastembed::OutputKey::ByName("last_hidden_state"),
];

#[cfg(windows)]
impl DirectMlOrtSession {
    fn new_directml(assets: DirectMlRuntimeAssets) -> Result<Self, String> {
        let tokenizer = load_local_tokenizer(&assets.tokenizer_files, LOCAL_EMBED_MAX_LENGTH)?;
        let mut attempt_errors = Vec::new();

        for device_id in directml_device_ids_to_try() {
            for optimization_level in [
                GraphOptimizationLevel::Level3,
                GraphOptimizationLevel::Level2,
                GraphOptimizationLevel::Level1,
                GraphOptimizationLevel::Disable,
            ] {
                let optimization_label = directml_optimization_label(optimization_level);
                match build_directml_ort_session(
                    &assets.onnx_file,
                    &assets.external_initializers,
                    device_id,
                    optimization_level,
                    assets.quantization != QuantizationMode::None,
                ) {
                    Ok(session) => {
                        let input_schema =
                            detect_directml_input_schema(session.inputs(), &assets.config_hints)?;
                        return Ok(Self {
                            tokenizer,
                            session,
                            route: LocalOrtExecutionRoute::DirectMl,
                            device_id: Some(device_id),
                            device_name: query_gpu_adapter_name_for_index(device_id)
                                .unwrap_or_else(|| "GPU".to_string()),
                            input_schema,
                            pooling: assets.pooling,
                            output_key: assets.output_key,
                            quantization: assets.quantization,
                        });
                    }
                    Err(error) => {
                        let attempt_error = format!(
                            "device_id={device_id}, optimization={optimization_label}, model_file={}: {}",
                            assets.model_file_path.display(),
                            error
                        );
                        attempt_errors.push(attempt_error);
                    }
                }
            }
        }

        Err(format!(
            "Failed to initialize GPU-DirectML session with compatible settings: {}",
            if attempt_errors.is_empty() {
                "no compatible adapter or session configuration succeeded".to_string()
            } else {
                attempt_errors.join(" | ")
            }
        ))
    }

    fn new_cpu(assets: DirectMlRuntimeAssets) -> Result<Self, String> {
        let tokenizer = load_local_tokenizer(&assets.tokenizer_files, LOCAL_EMBED_MAX_LENGTH)?;
        let mut attempt_errors = Vec::new();

        for optimization_level in [
            GraphOptimizationLevel::Level3,
            GraphOptimizationLevel::Level2,
            GraphOptimizationLevel::Level1,
            GraphOptimizationLevel::Disable,
        ] {
            let optimization_label = directml_optimization_label(optimization_level);
            match build_cpu_ort_session(
                &assets.onnx_file,
                &assets.external_initializers,
                optimization_level,
                assets.quantization != QuantizationMode::None,
            ) {
                Ok(session) => {
                    let input_schema =
                        detect_directml_input_schema(session.inputs(), &assets.config_hints)?;
                    return Ok(Self {
                        tokenizer,
                        session,
                        route: LocalOrtExecutionRoute::Cpu,
                        device_id: None,
                        device_name: current_cpu_device_name().unwrap_or_else(|| "CPU".to_string()),
                        input_schema,
                        pooling: assets.pooling,
                        output_key: assets.output_key,
                        quantization: assets.quantization,
                    });
                }
                Err(error) => attempt_errors.push(format!(
                    "optimization={optimization_label}, model_file={}: {}",
                    assets.model_file_path.display(),
                    error
                )),
            }
        }

        Err(format!(
            "Failed to initialize CPU ONNX session with compatible settings: {}",
            if attempt_errors.is_empty() {
                "no compatible session configuration succeeded".to_string()
            } else {
                attempt_errors.join(" | ")
            }
        ))
    }

    fn probe_dimension(&mut self) -> Result<usize, String> {
        self.embed_batch(&["dimension probe"], 1)
            .and_then(|vectors| {
                vectors
                    .first()
                    .map(Vec::len)
                    .filter(|dimension| *dimension > 0)
                    .ok_or_else(|| "Embedding runtime returned an empty vector".to_string())
            })
    }

    fn embed_batch(&mut self, texts: &[&str], batch_size: usize) -> Result<Vec<Vec<f32>>, String> {
        let effective_batch_size = if self.quantization == QuantizationMode::Dynamic {
            texts.len().max(1)
        } else {
            batch_size.min(texts.len().max(1))
        };
        let mut embeddings = Vec::with_capacity(texts.len());

        for batch in texts.chunks(effective_batch_size) {
            let inputs = batch.iter().copied().collect::<Vec<_>>();
            let encodings = self
                .tokenizer
                .encode_batch(inputs, true)
                .map_err(|error| format!("Failed to encode input batch: {error}"))?;
            let encoding_length = encodings
                .first()
                .ok_or_else(|| "Tokenizer returned empty encodings".to_string())?
                .len();
            let current_batch_size = batch.len();
            let max_size = encoding_length * current_batch_size;

            let mut ids_array = Vec::with_capacity(max_size);
            let mut mask_array = Vec::with_capacity(max_size);
            let mut type_ids_array = Vec::with_capacity(max_size);

            for encoding in &encodings {
                ids_array.extend(encoding.get_ids().iter().map(|value| *value as i64));
                mask_array.extend(
                    encoding
                        .get_attention_mask()
                        .iter()
                        .map(|value| *value as i64),
                );
                type_ids_array.extend(encoding.get_type_ids().iter().map(|value| *value as i64));
            }

            let input_ids_array =
                Array::from_shape_vec((current_batch_size, encoding_length), ids_array)
                    .map_err(|error| format!("Failed to build input_ids tensor: {error}"))?;
            let attention_mask_array =
                Array::from_shape_vec((current_batch_size, encoding_length), mask_array)
                    .map_err(|error| format!("Failed to build attention_mask tensor: {error}"))?;
            let token_type_ids_array =
                Array::from_shape_vec((current_batch_size, encoding_length), type_ids_array)
                    .map_err(|error| format!("Failed to build token_type_ids tensor: {error}"))?;
            let mut session_inputs = ort::inputs![
                self.input_schema.input_ids_name.as_str() => Value::from_array(input_ids_array)
                    .map_err(|error| format!("Failed to build ONNX input tensor: {error}"))?,
            ];

            if let Some(attention_mask_name) = self.input_schema.attention_mask_name.as_deref() {
                session_inputs.push((
                    attention_mask_name.into(),
                    Value::from_array(attention_mask_array.clone())
                        .map_err(|error| format!("Failed to build ONNX attention mask: {error}"))?
                        .into(),
                ));
            }

            if let Some(position_ids_name) = self.input_schema.position_ids_name.as_deref() {
                let position_ids_array = build_position_ids_array(&attention_mask_array);
                session_inputs.push((
                    position_ids_name.into(),
                    Value::from_array(position_ids_array)
                        .map_err(|error| format!("Failed to build position_ids value: {error}"))?
                        .into(),
                ));
            }

            if let Some(token_type_ids_name) = self.input_schema.token_type_ids_name.as_deref() {
                session_inputs.push((
                    token_type_ids_name.into(),
                    Value::from_array(token_type_ids_array)
                        .map_err(|error| format!("Failed to build token_type_ids value: {error}"))?
                        .into(),
                ));
            }

            for cache_input in &self.input_schema.past_key_values {
                let key_tensor = build_zero_past_key_value_tensor(
                    self.session.allocator(),
                    cache_input,
                    current_batch_size,
                )?;
                let value_tensor = build_zero_past_key_value_tensor(
                    self.session.allocator(),
                    cache_input,
                    current_batch_size,
                )?;
                session_inputs.push((cache_input.key_name.clone().into(), key_tensor.into()));
                session_inputs.push((cache_input.value_name.clone().into(), value_tensor.into()));
            }

            let outputs = self
                .session
                .run(session_inputs)
                .map_err(|error| format!("Failed to run local ONNX embedding session: {error}"))?
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect::<Vec<_>>();

            let pooled = pool_output_tensor(
                &outputs,
                &attention_mask_array,
                self.pooling.clone(),
                self.output_key.as_ref(),
            )?;
            for row in pooled.rows() {
                let slice = row
                    .as_slice()
                    .ok_or_else(|| "Failed to read pooled embedding row".to_string())?;
                embeddings.push(normalize_embedding_values(slice));
            }
        }

        Ok(embeddings)
    }
}

#[cfg(windows)]
fn resolve_directml_assets_for_fastembed_model(
    model_dir: &Path,
    model: &FastembedModel,
) -> Result<DirectMlRuntimeAssets, String> {
    let info = TextEmbedding::get_model_info(model)
        .map_err(|error| format!("Failed to resolve model info: {error}"))?;
    let model_code = info.model_code.clone();
    let cache_dir = effective_fastembed_cache_dir(model_dir);
    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir)
        .build()
        .map_err(|error| format!("Failed to open local Hugging Face cache: {error}"))?;
    let repo = api.model(model_code.clone());
    let model_file_path = repo.get(&info.model_file).map_err(|error| {
        format!(
            "Failed to resolve cached model file '{}': {}",
            info.model_file, error
        )
    })?;
    let tokenizer_files = load_tokenizer_files_from_repo(&repo)?;
    let onnx_file = std::fs::read(&model_file_path).map_err(|error| {
        format!(
            "Failed to read cached model file '{}': {}",
            model_file_path.display(),
            error
        )
    })?;

    let config_hints = parse_model_config_hints(&tokenizer_files.config_file);

    Ok(DirectMlRuntimeAssets {
        external_initializers: collect_manual_external_initializers(&model_file_path)?,
        model_file_path,
        onnx_file,
        tokenizer_files,
        config_hints,
        pooling: TextEmbedding::get_default_pooling_method(model)
            .map(manual_pooling_from_fastembed),
        output_key: info.output_key.clone(),
        quantization: TextEmbedding::get_quantization_mode(model),
    })
}

#[cfg(windows)]
fn resolve_directml_assets_for_manual_model(
    manual_model_dir: &Path,
    requested_model: &str,
    known_model: Option<&FastembedModel>,
) -> Result<DirectMlRuntimeAssets, String> {
    let model_file_path = find_manual_model_file(manual_model_dir).ok_or_else(|| {
        format!(
            "No ONNX model file was found in {}",
            manual_model_dir.display()
        )
    })?;
    let tokenizer_files = load_manual_tokenizer_files(manual_model_dir)?;
    let onnx_file = std::fs::read(&model_file_path).map_err(|error| {
        format!(
            "Failed to read ONNX model '{}': {}",
            model_file_path.display(),
            error
        )
    })?;
    let config_hints = parse_model_config_hints(&tokenizer_files.config_file);
    let pooling = resolve_manual_pooling(manual_model_dir, requested_model, known_model)?;
    let output_key = known_model
        .and_then(|model| TextEmbedding::get_model_info(model).ok())
        .and_then(|info| info.output_key.clone());
    let quantization = known_model
        .map(TextEmbedding::get_quantization_mode)
        .unwrap_or(QuantizationMode::None);

    Ok(DirectMlRuntimeAssets {
        external_initializers: collect_manual_external_initializers(&model_file_path)?,
        model_file_path,
        onnx_file,
        tokenizer_files,
        config_hints,
        pooling,
        output_key,
        quantization,
    })
}

#[cfg(windows)]
fn load_tokenizer_files_from_repo(repo: &ApiRepo) -> Result<TokenizerFiles, String> {
    Ok(TokenizerFiles {
        tokenizer_file: std::fs::read(
            repo.get("tokenizer.json")
                .map_err(|error| format!("Failed to resolve tokenizer.json from cache: {error}"))?,
        )
        .map_err(|error| format!("Failed to read tokenizer.json: {error}"))?,
        config_file: std::fs::read(
            repo.get("config.json")
                .map_err(|error| format!("Failed to resolve config.json from cache: {error}"))?,
        )
        .map_err(|error| format!("Failed to read config.json: {error}"))?,
        special_tokens_map_file: read_optional_repo_file(
            repo,
            "special_tokens_map.json",
            EMPTY_JSON_OBJECT,
        )?,
        tokenizer_config_file: std::fs::read(repo.get("tokenizer_config.json").map_err(
            |error| format!("Failed to resolve tokenizer_config.json from cache: {error}"),
        )?)
        .map_err(|error| format!("Failed to read tokenizer_config.json: {error}"))?,
    })
}

#[cfg(windows)]
fn read_optional_repo_file(
    repo: &ApiRepo,
    file_name: &str,
    default_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let Ok(path) = repo.get(file_name) else {
        return Ok(default_bytes.to_vec());
    };
    std::fs::read(&path).map_err(|error| format!("Failed to read {}: {error}", path.display()))
}

#[cfg(windows)]
fn load_local_tokenizer(
    tokenizer_files: &TokenizerFiles,
    max_length: usize,
) -> Result<Tokenizer, String> {
    let config: serde_json::Value = serde_json::from_slice(&tokenizer_files.config_file)
        .map_err(|_| "Failed to parse config.json".to_string())?;
    let special_tokens_map: serde_json::Value =
        serde_json::from_slice(&tokenizer_files.special_tokens_map_file)
            .map_err(|_| "Failed to parse special_tokens_map.json".to_string())?;
    let tokenizer_config: serde_json::Value =
        serde_json::from_slice(&tokenizer_files.tokenizer_config_file)
            .map_err(|_| "Failed to parse tokenizer_config.json".to_string())?;
    let mut tokenizer = Tokenizer::from_bytes(tokenizer_files.tokenizer_file.clone())
        .map_err(|error| format!("Failed to parse tokenizer.json: {error}"))?;

    let model_max_length = tokenizer_config["model_max_length"]
        .as_f64()
        .ok_or_else(|| "tokenizer_config.json is missing model_max_length".to_string())?
        as usize;
    let effective_max_length = max_length.min(model_max_length);
    let pad_id = config["pad_token_id"].as_u64().unwrap_or(0) as u32;
    let pad_token = tokenizer_config["pad_token"]
        .as_str()
        .ok_or_else(|| "tokenizer_config.json is missing pad_token".to_string())?
        .to_string();

    tokenizer = tokenizer
        .with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_token,
            pad_id,
            ..Default::default()
        }))
        .with_truncation(Some(TruncationParams {
            max_length: effective_max_length,
            ..Default::default()
        }))
        .map_err(|error| format!("Failed to configure tokenizer truncation: {error}"))?
        .clone()
        .into();

    let mut seen_special_tokens = HashSet::new();
    if let serde_json::Value::Object(root_object) = special_tokens_map {
        for value in root_object.values() {
            add_special_tokens_from_value(&mut tokenizer, value, &mut seen_special_tokens);
        }
    }
    if let serde_json::Value::Object(root_object) = tokenizer_config {
        for key in [
            "bos_token",
            "eos_token",
            "pad_token",
            "unk_token",
            "sep_token",
            "cls_token",
            "mask_token",
            "additional_special_tokens",
        ] {
            if let Some(value) = root_object.get(key) {
                add_special_tokens_from_value(&mut tokenizer, value, &mut seen_special_tokens);
            }
        }
    }

    Ok(tokenizer)
}

#[cfg(windows)]
fn add_special_tokens_from_value(
    tokenizer: &mut Tokenizer,
    value: &serde_json::Value,
    seen_tokens: &mut HashSet<String>,
) {
    match value {
        serde_json::Value::String(content) => {
            add_special_token(
                tokenizer,
                AddedToken {
                    content: content.clone().into(),
                    special: true,
                    ..Default::default()
                },
                seen_tokens,
            );
        }
        serde_json::Value::Array(items) => {
            for item in items {
                add_special_tokens_from_value(tokenizer, item, seen_tokens);
            }
        }
        serde_json::Value::Object(object) => {
            let Some(content) = object.get("content").and_then(|value| value.as_str()) else {
                return;
            };
            let mut token = AddedToken {
                content: content.to_string().into(),
                special: true,
                ..Default::default()
            };
            if let Some(single_word) = object.get("single_word").and_then(|value| value.as_bool()) {
                token.single_word = single_word;
            }
            if let Some(lstrip) = object.get("lstrip").and_then(|value| value.as_bool()) {
                token.lstrip = lstrip;
            }
            if let Some(rstrip) = object.get("rstrip").and_then(|value| value.as_bool()) {
                token.rstrip = rstrip;
            }
            if let Some(normalized) = object.get("normalized").and_then(|value| value.as_bool()) {
                token.normalized = normalized;
            }
            add_special_token(tokenizer, token, seen_tokens);
        }
        _ => {}
    }
}

#[cfg(windows)]
fn add_special_token(
    tokenizer: &mut Tokenizer,
    token: AddedToken,
    seen_tokens: &mut HashSet<String>,
) {
    let content = token.content.clone();
    if content.trim().is_empty() || !seen_tokens.insert(content) {
        return;
    }
    tokenizer.add_special_tokens(&[token]);
}

#[cfg(windows)]
fn build_directml_ort_session(
    onnx_file: &[u8],
    external_initializers: &[(String, Vec<u8>)],
    device_id: i32,
    optimization_level: GraphOptimizationLevel,
    enable_quant_qdq: bool,
) -> Result<Session, String> {
    preload_ort_helpers();

    let mut builder = Session::builder().map_err(|error| {
        format!(
            "Failed to create ONNX Runtime session builder: {}",
            normalize_runtime_error_message_with_debug(
                &error.to_string(),
                Some(&format!("{error:#?}")),
                "Session::builder failed"
            )
        )
    })?;
    builder = builder
        .with_execution_providers([DirectML::default()
            .with_device_id(device_id)
            .build()
            .error_on_failure()])
        .map_err(|error| {
            format!(
                "Failed to configure DirectML execution provider: {}",
                normalize_runtime_error_message_with_debug(
                    &error.to_string(),
                    Some(&format!("{error:#?}")),
                    "DirectML provider configuration failed"
                )
            )
        })?;
    builder = builder
        .with_optimization_level(optimization_level)
        .map_err(|error| format!("Failed to set graph optimization level: {error}"))?;
    builder = builder
        .with_memory_pattern(false)
        .map_err(|error| format!("Failed to disable memory pattern optimization: {error}"))?;
    builder = builder
        .with_parallel_execution(false)
        .map_err(|error| format!("Failed to disable parallel execution: {error}"))?;
    builder = builder
        .with_intra_threads(1)
        .map_err(|error| format!("Failed to set DirectML intra-op threads: {error}"))?;
    builder = builder
        .with_intra_op_spinning(false)
        .map_err(|error| format!("Failed to disable intra-op spinning: {error}"))?;
    builder = builder
        .with_inter_op_spinning(false)
        .map_err(|error| format!("Failed to disable inter-op spinning: {error}"))?;
    if enable_quant_qdq {
        builder = builder
            .with_quant_qdq(true)
            .map_err(|error| format!("Failed to enable QDQ support for DirectML: {error}"))?;
    }
    for (file_name, buffer) in external_initializers {
        builder = builder
            .with_external_initializer_file_in_memory(file_name, Cow::Owned(buffer.clone()))
            .map_err(|error| {
                format!(
                    "Failed to register external initializer '{}': {}",
                    file_name, error
                )
            })?;
    }
    builder.commit_from_memory(onnx_file).map_err(|error| {
        normalize_runtime_error_message_with_debug(
            &error.to_string(),
            Some(&format!("{error:#?}")),
            "DirectML session commit failed",
        )
    })
}

#[cfg(windows)]
fn build_cpu_ort_session(
    onnx_file: &[u8],
    external_initializers: &[(String, Vec<u8>)],
    optimization_level: GraphOptimizationLevel,
    enable_quant_qdq: bool,
) -> Result<Session, String> {
    preload_ort_helpers();

    let mut builder = Session::builder().map_err(|error| {
        format!(
            "Failed to create ONNX Runtime session builder: {}",
            normalize_runtime_error_message_with_debug(
                &error.to_string(),
                Some(&format!("{error:#?}")),
                "Session::builder failed"
            )
        )
    })?;
    builder = builder
        .with_optimization_level(optimization_level)
        .map_err(|error| format!("Failed to set graph optimization level: {error}"))?;
    builder = builder
        .with_memory_pattern(false)
        .map_err(|error| format!("Failed to disable memory pattern optimization: {error}"))?;
    builder = builder
        .with_parallel_execution(false)
        .map_err(|error| format!("Failed to disable parallel execution: {error}"))?;
    builder = builder
        .with_intra_threads(1)
        .map_err(|error| format!("Failed to set CPU intra-op threads: {error}"))?;
    builder = builder
        .with_intra_op_spinning(false)
        .map_err(|error| format!("Failed to disable intra-op spinning: {error}"))?;
    builder = builder
        .with_inter_op_spinning(false)
        .map_err(|error| format!("Failed to disable inter-op spinning: {error}"))?;
    if enable_quant_qdq {
        builder = builder
            .with_quant_qdq(true)
            .map_err(|error| format!("Failed to enable QDQ support for CPU runtime: {error}"))?;
    }
    for (file_name, buffer) in external_initializers {
        builder = builder
            .with_external_initializer_file_in_memory(file_name, Cow::Owned(buffer.clone()))
            .map_err(|error| {
                format!(
                    "Failed to register external initializer '{}': {}",
                    file_name, error
                )
            })?;
    }
    builder.commit_from_memory(onnx_file).map_err(|error| {
        normalize_runtime_error_message_with_debug(
            &error.to_string(),
            Some(&format!("{error:#?}")),
            "CPU session commit failed",
        )
    })
}

#[cfg(windows)]
fn directml_device_ids_to_try() -> Vec<i32> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory6, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
    };

    let factory: IDXGIFactory6 = match unsafe { CreateDXGIFactory1() } {
        Ok(factory) => factory,
        Err(_) => return vec![0],
    };

    let preferred_luid =
        unsafe { factory.EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE) }
            .ok()
            .and_then(|adapter: IDXGIAdapter1| unsafe { adapter.GetDesc1().ok() })
            .map(|desc| (desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart));

    let mut candidates = Vec::new();
    let mut index = 0;
    loop {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(_) => break,
        };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            index += 1;
            continue;
        };
        let luid = (desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart);
        candidates.push(DirectMlAdapterCandidate {
            index: index as i32,
            dedicated_video_memory: desc.DedicatedVideoMemory as u64,
            is_software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
            is_high_performance: preferred_luid == Some(luid),
        });
        index += 1;
    }

    prioritize_directml_adapter_indices(&candidates)
}

#[cfg(windows)]
fn directml_optimization_label(level: GraphOptimizationLevel) -> &'static str {
    match level {
        GraphOptimizationLevel::Disable => "disable",
        GraphOptimizationLevel::Level1 => "level1",
        GraphOptimizationLevel::Level2 => "level2",
        GraphOptimizationLevel::Level3 => "level3",
        GraphOptimizationLevel::All => "all",
    }
}

#[cfg(windows)]
fn collect_directml_adapter_diagnostics() -> Vec<EmbeddingRuntimeTestAdapterInfo> {
    enumerate_dxgi_adapters()
}

#[cfg(windows)]
fn enumerate_dxgi_adapters() -> Vec<EmbeddingRuntimeTestAdapterInfo> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory6, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
    };

    fn adapter_name(desc: &windows::Win32::Graphics::Dxgi::DXGI_ADAPTER_DESC1) -> String {
        let end = desc
            .Description
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(desc.Description.len());
        String::from_utf16_lossy(&desc.Description[..end])
    }

    let factory: IDXGIFactory6 = match unsafe { CreateDXGIFactory1() } {
        Ok(factory) => factory,
        Err(_) => return Vec::new(),
    };

    let preferred_luid =
        unsafe { factory.EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE) }
            .ok()
            .and_then(|adapter: IDXGIAdapter1| unsafe { adapter.GetDesc1().ok() })
            .map(|desc| (desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart));

    let mut adapters = Vec::new();
    let mut index = 0;
    loop {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(_) => break,
        };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            index += 1;
            continue;
        };
        let luid = (desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart);
        adapters.push(EmbeddingRuntimeTestAdapterInfo {
            index: index as i32,
            name: adapter_name(&desc),
            vendor_id: desc.VendorId,
            device_id: desc.DeviceId,
            dedicated_vram_bytes: desc.DedicatedVideoMemory as u64,
            is_software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
            is_high_performance: preferred_luid == Some(luid),
        });
        index += 1;
    }

    adapters
}

#[cfg(windows)]
fn query_gpu_adapter_name_for_index(adapter_index: i32) -> Option<String> {
    enumerate_dxgi_adapters()
        .into_iter()
        .find(|adapter| adapter.index == adapter_index)
        .map(|adapter| adapter.name)
}

#[cfg(windows)]
fn preferred_gpu_device_name() -> Option<String> {
    let adapters = enumerate_dxgi_adapters();
    let preferred = adapters
        .iter()
        .filter(|adapter| !adapter.is_software)
        .filter(|adapter| adapter.is_high_performance)
        .max_by(|left, right| {
            left.dedicated_vram_bytes
                .cmp(&right.dedicated_vram_bytes)
                .then_with(|| right.index.cmp(&left.index))
        })
        .map(|adapter| adapter.name.clone());
    if preferred.is_some() {
        return preferred;
    }

    adapters
        .into_iter()
        .filter(|adapter| !adapter.is_software)
        .max_by(|left, right| {
            left.dedicated_vram_bytes
                .cmp(&right.dedicated_vram_bytes)
                .then_with(|| right.index.cmp(&left.index))
        })
        .map(|adapter| adapter.name)
}

#[cfg(not(windows))]
fn preferred_gpu_device_name() -> Option<String> {
    None
}

#[cfg(windows)]
fn preferred_gpu_dedicated_vram_bytes() -> Option<u64> {
    let adapters = enumerate_dxgi_adapters();
    adapters
        .iter()
        .filter(|adapter| !adapter.is_software)
        .filter(|adapter| adapter.is_high_performance)
        .max_by(|left, right| {
            left.dedicated_vram_bytes
                .cmp(&right.dedicated_vram_bytes)
                .then_with(|| right.index.cmp(&left.index))
        })
        .map(|adapter| adapter.dedicated_vram_bytes)
        .or_else(|| {
            adapters
                .into_iter()
                .filter(|adapter| !adapter.is_software)
                .max_by(|left, right| {
                    left.dedicated_vram_bytes
                        .cmp(&right.dedicated_vram_bytes)
                        .then_with(|| right.index.cmp(&left.index))
                })
                .map(|adapter| adapter.dedicated_vram_bytes)
        })
}

#[cfg(not(windows))]
fn preferred_gpu_dedicated_vram_bytes() -> Option<u64> {
    None
}

#[cfg(windows)]
fn current_cpu_device_name() -> Option<String> {
    std::env::var("PROCESSOR_NAME")
        .ok()
        .or_else(|| std::env::var("PROCESSOR_IDENTIFIER").ok())
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
}

#[cfg(not(windows))]
fn current_cpu_device_name() -> Option<String> {
    None
}

#[cfg(windows)]
fn read_runtime_dll_version(path: &Path) -> Option<String> {
    use windows::core::{w, HSTRING};
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };

    let path = HSTRING::from(path.as_os_str().to_string_lossy().as_ref());
    let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
    if size == 0 {
        return None;
    }

    let mut buffer = vec![0u8; size as usize];
    unsafe {
        GetFileVersionInfoW(&path, None, size, buffer.as_mut_ptr().cast()).ok()?;
    }

    let mut version_ptr = std::ptr::null_mut();
    let mut version_len = 0u32;
    if !unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            w!("\\"),
            &mut version_ptr,
            &mut version_len,
        )
    }
    .as_bool()
    {
        return None;
    }
    if version_ptr.is_null() || version_len < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32 {
        return None;
    }

    let version = unsafe { &*(version_ptr as *const VS_FIXEDFILEINFO) };
    Some(format!(
        "{}.{}.{}.{}",
        version.dwFileVersionMS >> 16,
        version.dwFileVersionMS & 0xFFFF,
        version.dwFileVersionLS >> 16,
        version.dwFileVersionLS & 0xFFFF
    ))
}

#[cfg(windows)]
fn query_gpu_memory_bytes_for_adapter_index(adapter_index: i32) -> Option<u64> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIAdapter3, IDXGIFactory6,
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
    };

    let factory: IDXGIFactory6 = unsafe { CreateDXGIFactory1().ok()? };
    let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapters1(adapter_index as u32).ok()? };
    let adapter: IDXGIAdapter3 = adapter.cast().ok()?;
    let mut video_memory_info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
    unsafe {
        adapter
            .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut video_memory_info)
            .ok()?;
    }
    Some(video_memory_info.CurrentUsage)
}

#[cfg(windows)]
fn query_preferred_gpu_memory_bytes() -> Option<u64> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory6, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
    };

    let factory: IDXGIFactory6 = unsafe { CreateDXGIFactory1().ok()? };
    let adapter: IDXGIAdapter3 = unsafe {
        factory
            .EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE)
            .ok()?
    };
    let mut video_memory_info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
    unsafe {
        adapter
            .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut video_memory_info)
            .ok()?;
    }
    Some(video_memory_info.CurrentUsage)
}

#[cfg(windows)]
fn pool_output_tensor(
    outputs: &[(String, Value)],
    attention_mask_array: &Array2<i64>,
    pooling: Option<ManualPoolingMode>,
    output_key: Option<&fastembed::OutputKey>,
) -> Result<Array2<f32>, String> {
    let tensor = select_output_tensor(outputs, output_key)?;
    match pooling.unwrap_or_default() {
        ManualPoolingMode::Cls => pool_cls_output(&tensor),
        ManualPoolingMode::Mean => pool_mean_output(&tensor, attention_mask_array.clone()),
        ManualPoolingMode::LastToken => pool_last_token_output(&tensor, attention_mask_array),
    }
}

#[cfg(windows)]
fn select_output_tensor<'a>(
    outputs: &'a [(String, Value)],
    output_key: Option<&fastembed::OutputKey>,
) -> Result<ArrayView<'a, f32, Dim<IxDynImpl>>, String> {
    let selected = if let Some(output_key) = output_key {
        find_output_value(outputs, output_key)
    } else {
        DIRECTML_OUTPUT_PRECEDENCE
            .iter()
            .find_map(|key| find_output_value(outputs, key))
    }
    .ok_or_else(|| {
        format!(
            "No suitable output found in the model outputs. Available outputs: {:?}",
            outputs.iter().map(|(name, _)| name).collect::<Vec<_>>()
        )
    })?;

    selected
        .try_extract_array()
        .map_err(|error| format!("Failed to extract output tensor: {error}"))
}

#[cfg(windows)]
fn find_output_value<'a>(
    outputs: &'a [(String, Value)],
    key: &fastembed::OutputKey,
) -> Option<&'a Value> {
    match key {
        fastembed::OutputKey::OnlyOne => {
            if outputs.len() == 1 {
                outputs.first().map(|(_, value)| value)
            } else {
                None
            }
        }
        fastembed::OutputKey::ByOrder(index) => outputs.get(*index).map(|(_, value)| value),
        fastembed::OutputKey::ByName(name) => outputs
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value),
    }
}

#[cfg(windows)]
fn pool_cls_output(tensor: &ArrayView<f32, Dim<IxDynImpl>>) -> Result<Array2<f32>, String> {
    match tensor.dim().ndim() {
        2 => Ok(tensor.slice(s![.., ..]).to_owned()),
        3 => Ok(tensor.slice(s![.., 0, ..]).to_owned()),
        _ => Err(format!(
            "Invalid output shape {:?}. Expected 2D or 3D tensor.",
            tensor.dim()
        )),
    }
}

#[cfg(windows)]
fn pool_mean_output(
    token_embeddings: &ArrayView<f32, Dim<IxDynImpl>>,
    attention_mask_array: Array2<i64>,
) -> Result<Array2<f32>, String> {
    let attention_mask_shape = attention_mask_array.dim();
    if token_embeddings.dim().ndim() == 2 {
        return Ok(token_embeddings.slice(s![.., ..]).to_owned());
    }
    if token_embeddings.dim().ndim() != 3 {
        return Err(format!(
            "Invalid output shape {:?}. Expected 2D or 3D tensor.",
            token_embeddings.dim()
        ));
    }

    let token_embeddings = token_embeddings.slice(s![.., .., ..]);
    let attention_mask = attention_mask_array
        .insert_axis(Axis(2))
        .broadcast(token_embeddings.dim())
        .ok_or_else(|| {
            format!(
                "Could not broadcast attention mask from {:?} to {:?}",
                attention_mask_shape,
                token_embeddings.dim()
            )
        })?
        .mapv(|value| value as f32);
    let masked_tensor = &attention_mask * &token_embeddings;
    let sum = masked_tensor.sum_axis(Axis(1));
    let mask_sum = attention_mask.sum_axis(Axis(1));
    let mask_sum = mask_sum.mapv(|value| if value == 0.0 { 1.0 } else { value });
    Ok(&sum / &mask_sum)
}

#[cfg(windows)]
fn pool_last_token_output(
    token_embeddings: &ArrayView<f32, Dim<IxDynImpl>>,
    attention_mask_array: &Array2<i64>,
) -> Result<Array2<f32>, String> {
    if token_embeddings.dim().ndim() == 2 {
        return Ok(token_embeddings.slice(s![.., ..]).to_owned());
    }
    if token_embeddings.dim().ndim() != 3 {
        return Err(format!(
            "Invalid output shape {:?}. Expected 2D or 3D tensor.",
            token_embeddings.dim()
        ));
    }

    let token_embeddings = token_embeddings.slice(s![.., .., ..]);
    let (batch_size, sequence_length, hidden_size) = token_embeddings.dim();
    let mut pooled = Array2::<f32>::zeros((batch_size, hidden_size));

    for batch_index in 0..batch_size {
        let last_token_index = attention_mask_array
            .row(batch_index)
            .iter()
            .rposition(|value| *value != 0)
            .unwrap_or_else(|| sequence_length.saturating_sub(1));
        if last_token_index >= sequence_length {
            return Err(format!(
                "Computed last-token index {} exceeds sequence length {}",
                last_token_index, sequence_length
            ));
        }
        pooled
            .slice_mut(s![batch_index, ..])
            .assign(&token_embeddings.slice(s![batch_index, last_token_index, ..]));
    }

    Ok(pooled)
}

#[cfg(windows)]
fn normalize_embedding_values(values: &[f32]) -> Vec<f32> {
    let norm = (values.iter().map(|value| value * value).sum::<f32>()).sqrt();
    let epsilon = 1e-12;
    values
        .iter()
        .map(|value| value / (norm + epsilon))
        .collect()
}

fn authority_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{}]", host)
    } else {
        host.to_string()
    }
}

fn proxy_display_uri(uri: &Uri) -> String {
    let scheme = uri.scheme_str().unwrap_or("http");
    let host = uri.host().unwrap_or("<missing-host>");
    match uri.port_u16() {
        Some(port) => format!("{}://{}:{}", scheme, authority_host(host), port),
        None => format!("{}://{}", scheme, authority_host(host)),
    }
}

fn sanitize_proxy_display_url(raw: &str) -> String {
    if let Ok(url) = Url::parse(raw) {
        let scheme = url.scheme();
        let host = url.host_str().unwrap_or("<missing-host>");
        return match url.port_or_known_default() {
            Some(port) => format!("{}://{}:{}", scheme, authority_host(host), port),
            None => format!("{}://{}", scheme, authority_host(host)),
        };
    }

    if let Ok(uri) = raw.parse::<Uri>() {
        return proxy_display_uri(&uri);
    }

    if let Some((scheme, remainder)) = raw.split_once("://") {
        if let Some((_, host_part)) = remainder.rsplit_once('@') {
            return format!("{}://{}", scheme, host_part);
        }
    }

    raw.to_string()
}

fn system_proxy_match_uri(url: &str) -> Result<Uri, String> {
    let parsed: Uri = url
        .parse()
        .map_err(|e| format!("Failed to parse proxy target URL '{}': {}", url, e))?;
    let scheme = match parsed.scheme_str() {
        Some("http") | Some("https") => parsed
            .scheme_str()
            .ok_or_else(|| "Proxy target URL is missing a scheme".to_string())?,
        Some(other) => {
            return Err(format!(
                "Unsupported proxy target URL scheme '{}': {}",
                other, url
            ));
        }
        None => return Err(format!("Proxy target URL is missing a scheme: {}", url)),
    };
    let authority = parsed
        .authority()
        .cloned()
        .ok_or_else(|| format!("Proxy target URL is missing an authority: {}", url))?;
    let path_and_query = parsed
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| http::uri::PathAndQuery::from_static("/"));

    Uri::builder()
        .scheme(scheme)
        .authority(authority)
        .path_and_query(path_and_query)
        .build()
        .map_err(|e| format!("Failed to normalize proxy target URL '{}': {}", url, e))
}

fn system_proxy_env_key_for_target(target_uri: &Uri) -> &'static str {
    match target_uri.scheme_str() {
        Some("http") => "HTTP_PROXY",
        _ => "HTTPS_PROXY",
    }
}

fn configured_proxy_env_for_target(target_uri: &Uri) -> Option<(String, String)> {
    let env_keys = match target_uri.scheme_str() {
        Some("http") => ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"],
        _ => ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"],
    };

    for key in env_keys {
        if let Some(value) = std::env::var_os(key) {
            return Some((key.to_string(), value.to_string_lossy().to_string()));
        }
    }

    None
}

fn build_supported_ureq_proxy_url(
    proxy_uri: &Uri,
    auth: Option<(&str, &str)>,
) -> Result<Option<String>, String> {
    let scheme = match proxy_uri.scheme_str() {
        Some("http") => "http",
        Some("socks4") => "socks4",
        Some("socks4a") => "socks4a",
        Some("socks5") | Some("socks5h") => "socks5",
        Some("https") => return Ok(None),
        Some(other) => {
            return Err(format!(
                "Unsupported system proxy scheme for knowledge download: {}",
                other
            ));
        }
        None => return Err("System proxy URI is missing a scheme".to_string()),
    };

    let host = proxy_uri
        .host()
        .ok_or_else(|| "System proxy URI is missing host".to_string())?;
    let port = proxy_uri.port_u16().unwrap_or(match scheme {
        "http" => 80,
        "socks4" | "socks4a" | "socks5" => 1080,
        _ => 80,
    });
    let auth_prefix = auth
        .map(|(user, pass)| format!("{user}:{pass}@"))
        .unwrap_or_default();
    Ok(Some(format!(
        "{}://{}{}:{}",
        scheme,
        auth_prefix,
        authority_host(host),
        port
    )))
}

#[derive(Debug, Clone)]
struct ResolvedDownloadProxyRoute {
    proxy_state: String,
    proxy_env_key: Option<String>,
    proxy_url: Option<String>,
}

fn ensure_system_proxy_env_for_url(url: &str) -> Result<ResolvedDownloadProxyRoute, String> {
    let target_uri = system_proxy_match_uri(url)?;
    let env_key = system_proxy_env_key_for_target(&target_uri).to_string();

    if let Some((configured_env_key, configured_proxy)) =
        configured_proxy_env_for_target(&target_uri)
    {
        return Ok(ResolvedDownloadProxyRoute {
            proxy_state: "environment".to_string(),
            proxy_env_key: Some(configured_env_key),
            proxy_url: Some(sanitize_proxy_display_url(&configured_proxy)),
        });
    }

    let matcher = Matcher::from_system();
    let Some(proxy) = matcher.intercept(&target_uri) else {
        tracing::info!(
            log_module = "knowledge_index",
            target_url = %url,
            "Knowledge download did not find a matching system proxy"
        );
        return Ok(ResolvedDownloadProxyRoute {
            proxy_state: "direct".to_string(),
            proxy_env_key: None,
            proxy_url: None,
        });
    };
    let proxy_display = proxy_display_uri(proxy.uri());

    let Some(proxy_url) = build_supported_ureq_proxy_url(proxy.uri(), proxy.raw_auth())? else {
        tracing::warn!(
            log_module = "knowledge_index",
            target_url = %url,
            proxy_scheme = %proxy.uri().scheme_str().unwrap_or("unknown"),
            "Knowledge download found a system proxy, but the current hf-hub/ureq stack does not support that proxy scheme"
        );
        return Ok(ResolvedDownloadProxyRoute {
            proxy_state: "system_unsupported".to_string(),
            proxy_env_key: None,
            proxy_url: Some(proxy_display),
        });
    };

    tracing::info!(
        log_module = "knowledge_index",
        target_url = %url,
        proxy = %proxy_display,
        "Knowledge download is using the detected system proxy"
    );
    std::env::set_var(&env_key, &proxy_url);
    std::env::set_var(env_key.to_ascii_lowercase(), &proxy_url);
    Ok(ResolvedDownloadProxyRoute {
        proxy_state: "system".to_string(),
        proxy_env_key: Some(env_key),
        proxy_url: Some(proxy_display),
    })
}

pub fn prepare_local_model_download_network(
    download_source: &str,
) -> Result<EmbeddingDownloadNetworkStatus, String> {
    let source = normalize_local_model_download_source(download_source).to_string();
    let endpoint = download_source_endpoint(download_source).to_string();
    let proxy = ensure_system_proxy_env_for_url(&endpoint)?;
    Ok(EmbeddingDownloadNetworkStatus {
        source,
        endpoint,
        proxy_state: proxy.proxy_state,
        proxy_env_key: proxy.proxy_env_key,
        proxy_url: proxy.proxy_url,
    })
}

struct RemoteEmbeddingRuntime {
    client: ureq::Agent,
    endpoint: String,
    api_key: String,
    model: String,
    dimensions: u32,
    max_batch: usize,
    signature: EmbeddingBackendSignature,
}

impl RemoteEmbeddingRuntime {
    fn new(config: &EmbeddingConfig) -> Result<Self, String> {
        let endpoint = normalized_embedding_endpoint(&config.remote_endpoint)?;
        if config.remote_model.trim().is_empty() {
            return Err("Remote embedding model is required".into());
        }
        ensure_system_proxy_env_for_url(&endpoint)?;

        let client = ureq::AgentBuilder::new()
            .try_proxy_from_env(true)
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout_read(std::time::Duration::from_secs(60))
            .timeout_write(std::time::Duration::from_secs(60))
            .build();

        let max_batch = config.remote_max_batch.max(1) as usize;
        let mut runtime = Self {
            client,
            endpoint,
            api_key: config.remote_api_key.clone(),
            model: config.remote_model.clone(),
            dimensions: config.remote_dimensions,
            max_batch,
            signature: EmbeddingBackendSignature {
                runtime_name: "openai_compatible_remote".to_string(),
                model_id: config.remote_model.clone(),
                model_revision: config.remote_endpoint.clone(),
                device_route: "remote".to_string(),
                dimension: config.remote_dimensions as usize,
                normalize: true,
            },
        };

        let probe = runtime.request_embeddings(&["hello"])?;
        let probe_dim = probe.first().map(Vec::len).unwrap_or(0);
        if probe_dim == 0 {
            return Err("Remote embedding endpoint returned an empty vector".into());
        }
        runtime.signature.dimension = probe_dim;
        Ok(runtime)
    }

    fn request_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let mut body = json!({
            "model": self.model,
            "input": texts,
        });
        if self.dimensions > 0 {
            body["dimensions"] = json!(self.dimensions);
        }

        let mut req = self.client.post(&self.endpoint);
        if !self.api_key.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", self.api_key));
        }
        let response = req.set("Content-Type", "application/json").send_json(body);
        let text = match response {
            Ok(resp) => resp
                .into_string()
                .map_err(|e| format!("Failed to read embedding response: {}", e))?,
            Err(ureq::Error::Status(status, resp)) => {
                let detail = resp
                    .into_string()
                    .unwrap_or_default()
                    .chars()
                    .take(240)
                    .collect::<String>();
                return Err(format!(
                    "Remote embedding request failed (HTTP {}): {}",
                    status, detail
                ));
            }
            Err(ureq::Error::Transport(err)) => {
                return Err(format!("Remote embedding request failed: {}", err));
            }
        };

        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("Invalid embedding response: {}", e))?;
        let data = json["data"]
            .as_array()
            .ok_or_else(|| "Embedding response missing data array".to_string())?;

        let mut vectors = Vec::with_capacity(data.len());
        for item in data {
            let arr = item["embedding"]
                .as_array()
                .ok_or_else(|| "Embedding response missing vector data".to_string())?;
            let mut vector = Vec::with_capacity(arr.len());
            for value in arr {
                let float = value
                    .as_f64()
                    .ok_or_else(|| "Embedding vector contains a non-number".to_string())?;
                vector.push(float as f32);
            }
            vectors.push(vector);
        }

        if vectors.len() != texts.len() {
            return Err(format!(
                "Embedding response count mismatch: expected {}, got {}",
                texts.len(),
                vectors.len()
            ));
        }
        Ok(vectors)
    }
}

impl EmbeddingRuntime for RemoteEmbeddingRuntime {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut all_vectors = Vec::new();
        for chunk in texts.chunks(self.max_batch) {
            let mut batch = self.request_embeddings(chunk)?;
            all_vectors.append(&mut batch);
        }
        Ok(all_vectors)
    }

    fn backend_signature(&self) -> EmbeddingBackendSignature {
        self.signature.clone()
    }
}

pub fn config_path(library_dir: &Path) -> PathBuf {
    library_dir.join("knowledge_embedding_config.json")
}

pub fn managed_model_root(library_dir: &Path) -> PathBuf {
    library_dir.join("knowledge_models")
}

pub fn migrate_legacy_managed_model_root(
    library_dir: &Path,
    model_storage_dir: &Path,
) -> Result<(), String> {
    let legacy_root = dunce::canonicalize(managed_model_root(library_dir))
        .unwrap_or_else(|_| managed_model_root(library_dir));
    let shared_root = dunce::canonicalize(managed_model_root(model_storage_dir))
        .unwrap_or_else(|_| managed_model_root(model_storage_dir));
    if legacy_root == shared_root || !legacy_root.is_dir() {
        return Ok(());
    }
    copy_model_storage_entry(&legacy_root, &shared_root)
}

fn copy_model_storage_entry(source: &Path, target: &Path) -> Result<(), String> {
    if source.is_dir() {
        std::fs::create_dir_all(target).map_err(|e| {
            format!(
                "Failed to create model storage directory '{}': {}",
                target.display(),
                e
            )
        })?;
        for entry in std::fs::read_dir(source).map_err(|e| {
            format!(
                "Failed to read model storage directory '{}': {}",
                source.display(),
                e
            )
        })? {
            let entry = entry.map_err(|e| {
                format!(
                    "Failed to read model storage entry in '{}': {}",
                    source.display(),
                    e
                )
            })?;
            copy_model_storage_entry(&entry.path(), &target.join(entry.file_name()))?;
        }
        return Ok(());
    }

    if target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create model storage parent '{}': {}",
                parent.display(),
                e
            )
        })?;
    }
    std::fs::copy(source, target).map_err(|e| {
        format!(
            "Failed to copy model storage file '{}' -> '{}': {}",
            source.display(),
            target.display(),
            e
        )
    })?;
    Ok(())
}

pub fn load_config(library_dir: &Path) -> EmbeddingConfig {
    let path = config_path(library_dir);
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data)
            .map(sanitize_embedding_config)
            .unwrap_or_default(),
        Err(_) => EmbeddingConfig::default(),
    }
}

pub fn save_config(library_dir: &Path, config: &EmbeddingConfig) -> Result<(), String> {
    std::fs::create_dir_all(library_dir)
        .map_err(|e| format!("Failed to create embedding config dir: {}", e))?;
    let path = config_path(library_dir);
    let normalized = sanitize_embedding_config(config.clone());
    let data = serde_json::to_string_pretty(&normalized)
        .map_err(|e| format!("Failed to serialize embedding config: {}", e))?;
    std::fs::write(&path, data).map_err(|e| format!("Failed to write embedding config: {}", e))
}

fn sanitize_model_id(model_id: &str) -> String {
    model_id.replace('/', "--")
}

fn configured_local_model_dir(config: &EmbeddingConfig, model_root: &Path) -> PathBuf {
    let configured = config.local_model_path.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    model_root.join(sanitize_model_id(&config.local_model))
}

fn prepare_query_text<'a>(model_id: &str, query: &'a str) -> Cow<'a, str> {
    if let Some(prefix) = default_query_prefix_for_model(model_id) {
        let trimmed = query.trim_start();
        if trimmed.starts_with(prefix) {
            Cow::Borrowed(query)
        } else {
            Cow::Owned(format!("{}{}", prefix, trimmed))
        }
    } else {
        Cow::Borrowed(query)
    }
}

fn prepare_document_text<'a>(model_id: &str, text: &'a str) -> Cow<'a, str> {
    if let Some(prefix) = default_document_prefix_for_model(model_id) {
        let trimmed = text.trim_start();
        if trimmed.starts_with(prefix) {
            Cow::Borrowed(text)
        } else {
            Cow::Owned(format!("{}{}", prefix, trimmed))
        }
    } else {
        Cow::Borrowed(text)
    }
}

fn default_query_prefix_for_model(model_id: &str) -> Option<&'static str> {
    let normalized = model_id.trim().to_ascii_lowercase();
    if normalized.contains("qwen3-embedding") {
        return Some(
            "Instruct: Given a web search query, retrieve relevant passages that answer the query\nQuery:",
        );
    }
    None
}

fn default_document_prefix_for_model(_model_id: &str) -> Option<&'static str> {
    None
}

fn normalized_embedding_endpoint(endpoint: &str) -> Result<String, String> {
    let base = endpoint.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("Remote embedding endpoint is required".into());
    }
    if base.ends_with("/embeddings") {
        Ok(base.to_string())
    } else {
        Ok(format!("{}/embeddings", base))
    }
}

fn ensure_download_not_cancelled(
    cancel_requested: Option<&AtomicBool>,
) -> Result<(), EmbeddingDownloadError> {
    if cancel_requested
        .map(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
    {
        Err(EmbeddingDownloadError::Cancelled)
    } else {
        Ok(())
    }
}

fn temporary_download_path(target_path: &Path) -> PathBuf {
    let file_name = target_path
        .file_name()
        .map(|value| {
            let mut next = value.to_os_string();
            next.push(".part");
            next
        })
        .unwrap_or_else(|| "download.part".into());
    target_path.with_file_name(file_name)
}

fn remove_file_if_exists(path: &Path) {
    if path.is_file() {
        let _ = std::fs::remove_file(path);
    }
}

fn remove_dir_if_exists(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    }
}

fn prefetch_fastembed_model_with_cancel<F>(
    model: &FastembedModel,
    model_dir: &Path,
    download_source: &str,
    cancel_requested: Option<&AtomicBool>,
    on_progress: &mut F,
) -> Result<(), EmbeddingDownloadError>
where
    F: FnMut(EmbeddingActivationProgress),
{
    on_progress(EmbeddingActivationProgress::Stage {
        stage: "preparing",
        detail: Some(format!(
            "正在连接 {}",
            local_model_download_source_label(download_source)
        )),
    });
    ensure_download_not_cancelled(cancel_requested)?;

    let cache_dir = effective_fastembed_cache_dir(model_dir);
    let info = TextEmbedding::get_model_info(model).map_err(|error| {
        EmbeddingDownloadError::failed(format!(
            "Failed to read model metadata for {:?}: {}",
            model, error
        ))
    })?;
    let hf_endpoint = download_source_endpoint(download_source).to_string();
    ensure_system_proxy_env_for_url(&hf_endpoint).map_err(EmbeddingDownloadError::failed)?;

    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir.clone())
        .with_endpoint(hf_endpoint.clone())
        .with_progress(false)
        .build()
        .map_err(|error| {
            EmbeddingDownloadError::failed(format!(
                "Failed to initialize HuggingFace downloader: {}",
                error
            ))
        })?;
    let repo = api.model(info.model_code.clone());
    let cache_repo =
        Cache::new(cache_dir.clone()).repo(Repo::new(info.model_code.clone(), RepoType::Model));
    let repo_info = repo.info().map_err(|error| {
        EmbeddingDownloadError::failed(format!(
            "Failed to inspect Hugging Face repo '{}' for local model '{}': {}",
            info.model_code, info.model_code, error
        ))
    })?;
    let revision = repo_info.sha.trim().to_string();
    let target_snapshot_dir =
        fastembed_snapshot_dir_for_revision(&cache_dir, &info.model_code, &revision);

    if fastembed_model_cached(&cache_dir, model) {
        return Ok(());
    }

    remove_dir_if_exists(&target_snapshot_dir);

    let metadata_client =
        download_metadata_client(&hf_endpoint).map_err(EmbeddingDownloadError::failed)?;
    let pending_files: Vec<String> = required_fastembed_files(info)
        .into_iter()
        .filter(|file_name| cache_repo.get(file_name).is_none())
        .collect();
    let size_by_path = fetch_remote_repo_file_sizes(
        &metadata_client,
        &hf_endpoint,
        &info.model_code,
        &revision,
        pending_files.clone(),
    )
    .map_err(EmbeddingDownloadError::failed)?;
    let files_to_download = collect_remote_download_entries(pending_files, &size_by_path);

    if files_to_download.is_empty() {
        cache_repo.create_ref(&revision).map_err(|error| {
            EmbeddingDownloadError::failed(format!(
                "Failed to update local model cache ref '{}': {}",
                revision, error
            ))
        })?;
        return Ok(());
    }

    std::fs::create_dir_all(&target_snapshot_dir).map_err(|error| {
        EmbeddingDownloadError::failed(format!(
            "Failed to create model cache directory '{}': {}",
            target_snapshot_dir.display(),
            error
        ))
    })?;

    let overall_total_bytes: u64 = files_to_download.iter().map(|(_, size)| *size).sum();
    let mut completed_bytes = 0u64;
    let download_result = download_repo_files_to_directory(
        &metadata_client,
        &repo,
        &files_to_download,
        &target_snapshot_dir,
        overall_total_bytes,
        cancel_requested,
        &mut completed_bytes,
        on_progress,
    );

    match download_result {
        Ok(()) => {
            cache_repo.create_ref(&revision).map_err(|error| {
                EmbeddingDownloadError::failed(format!(
                    "Failed to update local model cache ref '{}': {}",
                    revision, error
                ))
            })?;
            Ok(())
        }
        Err(EmbeddingDownloadError::Cancelled) => {
            remove_dir_if_exists(&target_snapshot_dir);
            Err(EmbeddingDownloadError::Cancelled)
        }
        Err(error) => {
            remove_dir_if_exists(&target_snapshot_dir);
            Err(error)
        }
    }
}

fn download_custom_huggingface_model_with_cancel<F>(
    managed_model_id: &str,
    download_model_id: &str,
    managed_directory: &Path,
    download_source: &str,
    cancel_requested: Option<&AtomicBool>,
    on_progress: &mut F,
) -> Result<(), EmbeddingDownloadError>
where
    F: FnMut(EmbeddingActivationProgress),
{
    on_progress(EmbeddingActivationProgress::Stage {
        stage: "preparing",
        detail: Some(format!(
            "正在连接 {}",
            local_model_download_source_label(download_source)
        )),
    });
    ensure_download_not_cancelled(cancel_requested)?;

    let target_dir = managed_directory.join(sanitize_model_id(managed_model_id));
    if manual_model_files_ready(&target_dir) {
        return Ok(());
    }

    let hf_endpoint = download_source_endpoint(download_source).to_string();
    ensure_system_proxy_env_for_url(&hf_endpoint).map_err(EmbeddingDownloadError::failed)?;
    let api = ApiBuilder::new()
        .with_cache_dir(effective_fastembed_cache_dir(managed_directory))
        .with_endpoint(hf_endpoint.clone())
        .with_progress(false)
        .build()
        .map_err(|error| {
            EmbeddingDownloadError::failed(format!(
                "Failed to initialize HuggingFace downloader: {}",
                error
            ))
        })?;
    let repo = api.model(download_model_id.to_string());
    let repo_info = repo.info().map_err(|error| {
        EmbeddingDownloadError::failed(format!(
            "Failed to inspect Hugging Face repo '{}' for local model '{}': {}",
            download_model_id, managed_model_id, error
        ))
    })?;
    let revision = repo_info.sha.trim().to_string();
    let siblings: Vec<String> = repo_info
        .siblings
        .into_iter()
        .map(|item| item.rfilename)
        .collect();

    let model_file = resolve_repo_model_file(&siblings).ok_or_else(|| {
        EmbeddingDownloadError::failed(format!(
            "Hugging Face repo '{}' for local model '{}' does not contain a supported ONNX embedding model file",
            download_model_id, managed_model_id
        ))
    })?;

    let mut files_to_download = vec![model_file.clone()];
    for file_name in REQUIRED_LOCAL_TOKENIZER_FILE_NAMES {
        let resolved = resolve_repo_named_file(&siblings, file_name).ok_or_else(|| {
            EmbeddingDownloadError::failed(format!(
                "Hugging Face repo '{}' for local model '{}' is missing required file '{}'",
                download_model_id, managed_model_id, file_name
            ))
        })?;
        files_to_download.push(resolved);
    }
    for file_name in OPTIONAL_LOCAL_TOKENIZER_FILE_NAMES {
        if let Some(resolved) = resolve_repo_named_file(&siblings, file_name) {
            files_to_download.push(resolved);
        }
    }

    for optional_path in OPTIONAL_LOCAL_MODEL_FILE_NAMES {
        if let Some(path) = resolve_repo_exact_file(&siblings, optional_path) {
            files_to_download.push(path);
        }
    }
    for optional_path in OPTIONAL_LOCAL_MODEL_SENTENCE_TRANSFORMER_FILES {
        if let Some(path) = resolve_repo_exact_file(&siblings, optional_path) {
            files_to_download.push(path);
        }
    }

    files_to_download.extend(collect_repo_external_initializer_paths(
        &siblings,
        &model_file,
    ));
    files_to_download.sort();
    files_to_download.dedup();

    let metadata_client =
        download_metadata_client(&hf_endpoint).map_err(EmbeddingDownloadError::failed)?;
    let size_by_path = fetch_remote_repo_file_sizes(
        &metadata_client,
        &hf_endpoint,
        download_model_id,
        &revision,
        files_to_download.clone(),
    )
    .map_err(EmbeddingDownloadError::failed)?;
    let files_to_download = collect_remote_download_entries(files_to_download, &size_by_path);
    let overall_total_bytes: u64 = files_to_download.iter().map(|(_, size)| *size).sum();
    let mut completed_bytes = 0u64;

    remove_dir_if_exists(&target_dir);
    std::fs::create_dir_all(&target_dir).map_err(|error| {
        EmbeddingDownloadError::failed(format!(
            "Failed to create local model directory '{}': {}",
            target_dir.display(),
            error
        ))
    })?;

    let download_result = download_repo_files_to_directory(
        &metadata_client,
        &repo,
        &files_to_download,
        &target_dir,
        overall_total_bytes,
        cancel_requested,
        &mut completed_bytes,
        on_progress,
    );

    match download_result {
        Ok(()) => {
            write_managed_local_model_metadata(&target_dir, managed_model_id)
                .map_err(EmbeddingDownloadError::failed)?;
            let inspection = inspect_local_model_directory(&target_dir);
            if inspection.ready {
                Ok(())
            } else {
                Err(EmbeddingDownloadError::failed(format!(
                    "Downloaded Hugging Face repo '{}' for local model '{}' but the local model directory is still missing required files: {}",
                    download_model_id,
                    managed_model_id,
                    inspection.missing_files.join(", ")
                )))
            }
        }
        Err(EmbeddingDownloadError::Cancelled) => {
            remove_dir_if_exists(&target_dir);
            Err(EmbeddingDownloadError::Cancelled)
        }
        Err(error) => {
            remove_dir_if_exists(&target_dir);
            Err(error)
        }
    }
}

fn required_fastembed_files(info: &fastembed::ModelInfo<FastembedModel>) -> Vec<String> {
    let mut files = vec![info.model_file.clone()];
    files.extend(
        REQUIRED_LOCAL_TOKENIZER_FILE_NAMES
            .iter()
            .map(|file_name| (*file_name).to_string()),
    );
    files.extend(info.additional_files.clone());
    files
}

fn fastembed_repo_dir(cache_root: &Path, repo_id: &str) -> PathBuf {
    cache_root.join(Repo::new(repo_id.to_string(), RepoType::Model).folder_name())
}

fn fastembed_snapshot_dir_for_revision(
    cache_root: &Path,
    repo_id: &str,
    revision: &str,
) -> PathBuf {
    fastembed_repo_dir(cache_root, repo_id)
        .join("snapshots")
        .join(revision)
}

fn effective_fastembed_cache_dir(default_dir: &Path) -> PathBuf {
    std::env::var("HF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_dir.to_path_buf())
}

fn download_metadata_client(endpoint: &str) -> Result<ureq::Agent, String> {
    ensure_system_proxy_env_for_url(endpoint)?;
    Ok(ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .redirects(10)
        .build())
}

#[derive(Debug, Clone, Deserialize)]
struct HubRepoTreeEntry {
    #[serde(rename = "type")]
    entry_type: String,
    path: String,
    #[serde(default)]
    size: Option<u64>,
}

fn collect_remote_download_entries(
    file_names: Vec<String>,
    size_by_path: &HashMap<String, u64>,
) -> Vec<(String, u64)> {
    file_names
        .into_iter()
        .map(|file_name| {
            let size = size_by_path.get(&file_name).copied().unwrap_or_else(|| {
                eprintln!(
                    "[Knowledge] repo metadata did not include size for model file '{}'; continuing with streaming progress",
                    file_name
                );
                0
            });
            (file_name, size)
        })
        .collect()
}

fn fetch_remote_repo_file_sizes(
    client: &ureq::Agent,
    endpoint: &str,
    repo_id: &str,
    revision: &str,
    file_names: Vec<String>,
) -> Result<HashMap<String, u64>, String> {
    let required_paths: HashSet<String> = file_names.into_iter().collect();
    if required_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let mut size_by_path = HashMap::new();
    let mut next_url = Some(build_remote_repo_tree_url(endpoint, repo_id, revision)?);
    let mut page_count = 0usize;

    while let Some(url) = next_url.take() {
        page_count += 1;
        if page_count > 64 {
            return Err("Too many pages while loading Hugging Face repo tree metadata".to_string());
        }

        let response = client.get(&url).call().map_err(|error| {
            format!(
                "Failed to inspect Hugging Face repo tree metadata for '{}': {}",
                repo_id, error
            )
        })?;
        let next_link = response.header("link").map(str::to_string);
        let entries: Vec<HubRepoTreeEntry> = serde_json::from_reader(response.into_reader())
            .map_err(|error| {
                format!(
                    "Failed to parse Hugging Face repo tree metadata for '{}': {}",
                    repo_id, error
                )
            })?;

        for entry in entries {
            if entry.entry_type != "file" {
                continue;
            }
            if !required_paths.contains(&entry.path) {
                continue;
            }
            size_by_path.insert(entry.path, entry.size.unwrap_or(0));
        }

        if size_by_path.len() >= required_paths.len() {
            break;
        }

        next_url = match next_link {
            Some(link) => extract_next_link(&link)
                .map(|value| resolve_next_link_url(endpoint, value))
                .transpose()?,
            None => None,
        };
    }

    Ok(size_by_path)
}

fn build_remote_repo_tree_url(
    endpoint: &str,
    repo_id: &str,
    revision: &str,
) -> Result<String, String> {
    let mut url = Url::parse(endpoint)
        .map_err(|error| format!("Invalid Hugging Face endpoint '{}': {}", endpoint, error))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| format!("Invalid Hugging Face endpoint '{}'", endpoint))?;
        segments.pop_if_empty();
        segments.push("api");
        segments.push("models");
        for segment in repo_id.split('/') {
            segments.push(segment);
        }
        segments.push("tree");
        segments.push(revision);
    }
    url.query_pairs_mut().append_pair("recursive", "1");
    Ok(url.into())
}

fn extract_next_link(link_header: &str) -> Option<&str> {
    for part in link_header.split(',') {
        let trimmed = part.trim();
        if !trimmed.contains("rel=\"next\"") {
            continue;
        }
        let start = trimmed.find('<')? + 1;
        let end = trimmed[start..].find('>')? + start;
        return Some(&trimmed[start..end]);
    }
    None
}

fn resolve_next_link_url(endpoint: &str, next_link: &str) -> Result<String, String> {
    if Url::parse(next_link).is_ok() {
        return Ok(next_link.to_string());
    }
    Url::parse(endpoint)
        .map_err(|error| format!("Invalid Hugging Face endpoint '{}': {}", endpoint, error))?
        .join(next_link)
        .map(|url| url.into())
        .map_err(|error| {
            format!(
                "Invalid Hugging Face pagination link '{}': {}",
                next_link, error
            )
        })
}

fn download_repo_files_to_directory<F>(
    client: &ureq::Agent,
    repo: &ApiRepo,
    files: &[(String, u64)],
    target_root: &Path,
    overall_total_bytes: u64,
    cancel_requested: Option<&AtomicBool>,
    completed_bytes: &mut u64,
    on_progress: &mut F,
) -> Result<(), EmbeddingDownloadError>
where
    F: FnMut(EmbeddingActivationProgress),
{
    for (relative_path, expected_size) in files {
        ensure_download_not_cancelled(cancel_requested)?;
        on_progress(EmbeddingActivationProgress::Stage {
            stage: "downloading_model",
            detail: Some("正在下载模型文件".to_string()),
        });
        let mut progress = AggregatedDownloadProgress::new(
            relative_path,
            overall_total_bytes,
            completed_bytes,
            on_progress,
        );
        let target_path = join_relative_path(target_root, relative_path);
        download_remote_file_to_path(
            client,
            &repo.url(relative_path),
            &target_path,
            relative_path,
            *expected_size,
            cancel_requested,
            &mut progress,
        )?;
    }
    Ok(())
}

fn download_remote_file_to_path<F>(
    client: &ureq::Agent,
    url: &str,
    target_path: &Path,
    display_name: &str,
    expected_size: u64,
    cancel_requested: Option<&AtomicBool>,
    progress: &mut AggregatedDownloadProgress<'_, F>,
) -> Result<(), EmbeddingDownloadError>
where
    F: FnMut(EmbeddingActivationProgress),
{
    ensure_download_not_cancelled(cancel_requested)?;
    if let Some(parent_dir) = target_path.parent() {
        std::fs::create_dir_all(parent_dir).map_err(|error| {
            EmbeddingDownloadError::failed(format!(
                "Failed to create local model file directory '{}': {}",
                parent_dir.display(),
                error
            ))
        })?;
    }

    let temp_path = temporary_download_path(target_path);
    remove_file_if_exists(&temp_path);

    let response = client.get(url).call().map_err(|error| {
        EmbeddingDownloadError::failed(format!(
            "Failed to download model file '{}': {}",
            display_name, error
        ))
    })?;
    let response_size =
        parse_u64_header(response.header("content-length")).unwrap_or(expected_size);
    progress.start_file(display_name, response_size);

    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(&temp_path).map_err(|error| {
        EmbeddingDownloadError::failed(format!(
            "Failed to create temporary model file '{}': {}",
            temp_path.display(),
            error
        ))
    })?;

    let mut buffer = [0u8; 64 * 1024];
    let result = (|| -> Result<(), EmbeddingDownloadError> {
        loop {
            ensure_download_not_cancelled(cancel_requested)?;
            let read = reader.read(&mut buffer).map_err(|error| {
                EmbeddingDownloadError::failed(format!(
                    "Failed to read model file stream '{}': {}",
                    display_name, error
                ))
            })?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read]).map_err(|error| {
                EmbeddingDownloadError::failed(format!(
                    "Failed to write model file '{}': {}",
                    temp_path.display(),
                    error
                ))
            })?;
            progress.advance(read as u64);
        }
        file.flush().map_err(|error| {
            EmbeddingDownloadError::failed(format!(
                "Failed to flush model file '{}': {}",
                temp_path.display(),
                error
            ))
        })?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            progress.finish_file();
            std::fs::rename(&temp_path, target_path).map_err(|error| {
                EmbeddingDownloadError::failed(format!(
                    "Failed to finalize model file '{}': {}",
                    target_path.display(),
                    error
                ))
            })?;
            Ok(())
        }
        Err(error) => {
            remove_file_if_exists(&temp_path);
            Err(error)
        }
    }
}

fn parse_u64_header(value: Option<&str>) -> Option<u64> {
    value?.trim().parse::<u64>().ok()
}

fn resolve_repo_exact_file(siblings: &[String], relative_path: &str) -> Option<String> {
    siblings
        .iter()
        .find(|path| path.eq_ignore_ascii_case(relative_path))
        .cloned()
}

fn resolve_repo_named_file(siblings: &[String], file_name: &str) -> Option<String> {
    siblings
        .iter()
        .filter(|path| {
            Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case(file_name))
                .unwrap_or(false)
        })
        .min_by_key(|path| path.len())
        .cloned()
}

fn resolve_repo_model_file(siblings: &[String]) -> Option<String> {
    for candidate in LOCAL_MODEL_FILE_CANDIDATES {
        if let Some(path) = resolve_repo_exact_file(siblings, candidate) {
            return Some(path);
        }
    }

    siblings
        .iter()
        .filter(|path| path.to_ascii_lowercase().ends_with(".onnx"))
        .min_by_key(|path| path.len())
        .cloned()
}

fn collect_repo_external_initializer_paths(
    siblings: &[String],
    model_file_path: &str,
) -> Vec<String> {
    let model_path = Path::new(model_file_path);
    let Some(model_parent) = model_path.parent() else {
        return Vec::new();
    };

    siblings
        .iter()
        .filter(|path| path.as_str() != model_file_path)
        .filter(|path| Path::new(path).parent() == Some(model_parent))
        .filter(|path| {
            let Some(file_name) = Path::new(path).file_name().and_then(|value| value.to_str())
            else {
                return false;
            };
            let is_onnx = Path::new(path)
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("onnx"))
                .unwrap_or(false);
            !is_onnx && !should_skip_manual_initializer(file_name)
        })
        .cloned()
        .collect()
}

fn supported_embedding_presets() -> &'static [SupportedEmbeddingPreset] {
    &[
        SupportedEmbeddingPreset {
            id: "Qwen/Qwen3-Embedding-4B",
            label: "Qwen/Qwen3-Embedding-4B",
            dimensions: 2560,
            download_kind: EmbeddingPresetDownloadKind::HuggingFace,
            download_model_id: Some("onnx-community/Qwen3-Embedding-4B-ONNX"),
        },
        SupportedEmbeddingPreset {
            id: "Qwen/Qwen3-Embedding-0.6B",
            label: "Qwen/Qwen3-Embedding-0.6B",
            dimensions: 1024,
            download_kind: EmbeddingPresetDownloadKind::HuggingFace,
            download_model_id: Some("onnx-community/Qwen3-Embedding-0.6B-ONNX"),
        },
        SupportedEmbeddingPreset {
            id: "Alibaba-NLP/gte-multilingual-base",
            label: "Alibaba-NLP/gte-multilingual-base",
            dimensions: 768,
            download_kind: EmbeddingPresetDownloadKind::HuggingFace,
            download_model_id: Some("onnx-community/gte-multilingual-base"),
        },
        SupportedEmbeddingPreset {
            id: "jinaai/jina-embeddings-v5-text-small-retrieval",
            label: "jinaai/jina-embeddings-v5-text-small-retrieval",
            dimensions: 1024,
            download_kind: EmbeddingPresetDownloadKind::HuggingFace,
            download_model_id: Some("jinaai/jina-embeddings-v5-text-small-retrieval"),
        },
        SupportedEmbeddingPreset {
            id: "jinaai/jina-embeddings-v5-text-nano-retrieval",
            label: "jinaai/jina-embeddings-v5-text-nano-retrieval",
            dimensions: 768,
            download_kind: EmbeddingPresetDownloadKind::HuggingFace,
            download_model_id: Some("jinaai/jina-embeddings-v5-text-nano-retrieval"),
        },
        SupportedEmbeddingPreset {
            id: "Qwen/Qwen3-Embedding-8B",
            label: "Qwen/Qwen3-Embedding-8B",
            dimensions: 4096,
            download_kind: EmbeddingPresetDownloadKind::HuggingFace,
            download_model_id: Some("onnx-community/Qwen3-Embedding-8B-ONNX"),
        },
        SupportedEmbeddingPreset {
            id: "BAAI/bge-m3",
            label: "BAAI/bge-m3",
            dimensions: 1024,
            download_kind: EmbeddingPresetDownloadKind::Fastembed,
            download_model_id: None,
        },
    ]
}

fn known_fastembed_model_ids() -> &'static [&'static str] {
    &[
        "sentence-transformers/all-MiniLM-L6-v2",
        "BAAI/bge-small-en-v1.5",
        "BAAI/bge-base-en-v1.5",
        "nomic-ai/nomic-embed-text-v1.5",
        "mixedbread-ai/mxbai-embed-large-v1",
        "BAAI/bge-small-zh-v1.5",
        "BAAI/bge-large-zh-v1.5",
        "BAAI/bge-m3",
        "intfloat/multilingual-e5-large",
        "jinaai/jina-embeddings-v2-base-en",
        "Alibaba-NLP/gte-base-en-v1.5",
        "Alibaba-NLP/gte-large-en-v1.5",
    ]
}

fn supported_embedding_preset(model_id: &str) -> Option<&'static SupportedEmbeddingPreset> {
    let trimmed = model_id.trim();
    supported_embedding_presets()
        .iter()
        .find(|preset| preset.id == trimmed)
}

fn supported_embedding_preset_order(model_id: &str) -> Option<usize> {
    let trimmed = model_id.trim();
    supported_embedding_presets()
        .iter()
        .position(|preset| preset.id == trimmed)
}

fn supported_embedding_preset_download_model_id(preset: &SupportedEmbeddingPreset) -> &'static str {
    preset.download_model_id.unwrap_or(preset.id)
}

fn supported_embedding_preset_downloaded(
    preset: &SupportedEmbeddingPreset,
    cache_root: &Path,
    managed_directory: &Path,
) -> bool {
    match preset.download_kind {
        EmbeddingPresetDownloadKind::Fastembed => fastembed_model_for_id(preset.id)
            .map(|model| fastembed_model_cached(cache_root, &model))
            .unwrap_or(false),
        EmbeddingPresetDownloadKind::HuggingFace => {
            manual_model_files_ready(&managed_directory.join(sanitize_model_id(preset.id)))
        }
    }
}

fn model_dimension_for_id(model_id: &str) -> usize {
    if let Some(preset) = supported_embedding_preset(model_id) {
        return preset.dimensions;
    }
    fastembed_model_for_id(model_id)
        .and_then(|model| fastembed_model_dimension(&model))
        .unwrap_or(0)
}

fn fastembed_model_for_id(model_id: &str) -> Option<FastembedModel> {
    match model_id {
        "sentence-transformers/all-MiniLM-L6-v2" => Some(FastembedModel::AllMiniLML6V2),
        "BAAI/bge-small-en-v1.5" => Some(FastembedModel::BGESmallENV15),
        "BAAI/bge-base-en-v1.5" => Some(FastembedModel::BGEBaseENV15),
        "nomic-ai/nomic-embed-text-v1.5" => Some(FastembedModel::NomicEmbedTextV15),
        "mixedbread-ai/mxbai-embed-large-v1" => Some(FastembedModel::MxbaiEmbedLargeV1),
        "BAAI/bge-small-zh-v1.5" => Some(FastembedModel::BGESmallZHV15),
        "BAAI/bge-large-zh-v1.5" => Some(FastembedModel::BGELargeZHV15),
        "BAAI/bge-m3" => Some(FastembedModel::BGEM3),
        "intfloat/multilingual-e5-large" => Some(FastembedModel::MultilingualE5Large),
        "jinaai/jina-embeddings-v2-base-en" => Some(FastembedModel::JinaEmbeddingsV2BaseEN),
        "Alibaba-NLP/gte-base-en-v1.5" => Some(FastembedModel::GTEBaseENV15),
        "Alibaba-NLP/gte-large-en-v1.5" => Some(FastembedModel::GTELargeENV15),
        _ => None,
    }
}

fn fastembed_model_dimension(model: &FastembedModel) -> Option<usize> {
    TextEmbedding::get_model_info(model)
        .ok()
        .map(|info| info.dim)
}

fn fastembed_model_code(model: &FastembedModel) -> Option<String> {
    TextEmbedding::get_model_info(model)
        .ok()
        .map(|info| info.model_code.clone())
}

fn fastembed_model_cache_directory(cache_root: &Path, model: &FastembedModel) -> Option<PathBuf> {
    let info = TextEmbedding::get_model_info(model).ok()?;
    let repo_dir = cache_root.join(format!("models--{}", info.model_code.replace('/', "--")));
    if !repo_dir.exists() {
        return None;
    }

    let refs_main = repo_dir.join("refs").join("main");
    if let Ok(revision) = std::fs::read_to_string(&refs_main) {
        let snapshot_dir = repo_dir.join("snapshots").join(revision.trim());
        if snapshot_dir.exists() {
            return Some(snapshot_dir);
        }
    }

    let snapshots_dir = repo_dir.join("snapshots");
    if let Ok(entries) = std::fs::read_dir(&snapshots_dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                return Some(path);
            }
        }
    }

    Some(repo_dir)
}

fn fastembed_model_cached(cache_root: &Path, model: &FastembedModel) -> bool {
    let Ok(info) = TextEmbedding::get_model_info(model) else {
        return false;
    };
    let repo_dir = fastembed_model_cache_directory(cache_root, model).unwrap_or_else(|| {
        cache_root.join(format!("models--{}", info.model_code.replace('/', "--")))
    });
    if !repo_dir.exists() {
        return false;
    }

    let mut required_files = vec![info.model_file.clone()];
    required_files.extend(
        REQUIRED_LOCAL_TOKENIZER_FILE_NAMES
            .iter()
            .map(|file_name| (*file_name).to_string()),
    );
    required_files.extend(info.additional_files.clone());

    let mut found = vec![false; required_files.len()];
    for entry in WalkDir::new(&repo_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.path().is_file() {
            continue;
        }
        for (index, relative_path) in required_files.iter().enumerate() {
            if !found[index] && path_ends_with_components(entry.path(), relative_path) {
                found[index] = true;
            }
        }
        if found.iter().all(|present| *present) {
            return true;
        }
    }

    false
}

fn scan_managed_manual_local_models(managed_directory: &Path) -> Vec<EmbeddingAvailableLocalModel> {
    let Ok(entries) = std::fs::read_dir(managed_directory) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let dir_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if is_hf_cache_directory_name(dir_name) {
                return None;
            }

            let metadata = load_managed_local_model_metadata(&path);
            let metadata_model_id = metadata.as_ref().map(|value| value.model_id.trim());
            if metadata_model_id
                .and_then(supported_embedding_preset)
                .is_some()
                || supported_embedding_presets()
                    .iter()
                    .any(|preset| sanitize_model_id(preset.id) == dir_name)
            {
                return None;
            }
            let inspection = inspect_local_model_directory(&path);
            if !inspection.ready {
                return None;
            }

            let model_id = metadata
                .as_ref()
                .map(|value| value.model_id.clone())
                .unwrap_or_else(|| inspection.label.clone());
            Some(EmbeddingAvailableLocalModel {
                model_id: model_id.clone(),
                label: metadata
                    .as_ref()
                    .map(|value| value.label.clone())
                    .unwrap_or_else(|| inspection.label.clone()),
                local_model_path: inspection.path,
                dimensions: model_dimension_for_id(&model_id),
            })
        })
        .collect()
}

fn scan_cached_fastembed_models(cache_root: &Path) -> Vec<EmbeddingAvailableLocalModel> {
    known_fastembed_model_ids()
        .iter()
        .filter_map(|model_id| {
            let model = fastembed_model_for_id(model_id)?;
            if !fastembed_model_cached(cache_root, &model) {
                return None;
            }
            Some(EmbeddingAvailableLocalModel {
                model_id: (*model_id).to_string(),
                label: (*model_id).to_string(),
                local_model_path: String::new(),
                dimensions: model_dimension_for_id(model_id),
            })
        })
        .collect()
}

pub fn local_model_catalog(model_storage_dir: &Path) -> EmbeddingLocalModelCatalog {
    let managed_directory = managed_model_root(model_storage_dir);
    let cache_root = effective_fastembed_cache_dir(&managed_directory);
    let presets: Vec<_> = supported_embedding_presets()
        .iter()
        .map(|preset| EmbeddingModelPreset {
            id: preset.id.to_string(),
            label: preset.label.to_string(),
            downloaded: supported_embedding_preset_downloaded(
                preset,
                &cache_root,
                &managed_directory,
            ),
            dimensions: preset.dimensions,
        })
        .collect();
    let mut available_models: Vec<_> = presets
        .iter()
        .filter(|preset| preset.downloaded)
        .map(|preset| EmbeddingAvailableLocalModel {
            model_id: preset.id.clone(),
            label: preset.label.clone(),
            local_model_path: String::new(),
            dimensions: preset.dimensions,
        })
        .collect();
    let mut seen_models: HashSet<(String, String)> = available_models
        .iter()
        .map(|model| (model.model_id.clone(), model.local_model_path.clone()))
        .collect();
    for model in scan_cached_fastembed_models(&cache_root) {
        let key = (model.model_id.clone(), model.local_model_path.clone());
        if seen_models.insert(key) {
            available_models.push(model);
        }
    }
    for model in scan_managed_manual_local_models(&managed_directory) {
        let key = (model.model_id.clone(), model.local_model_path.clone());
        if seen_models.insert(key) {
            available_models.push(model);
        }
    }
    available_models.sort_by(|left, right| {
        supported_embedding_preset_order(&left.model_id)
            .unwrap_or(usize::MAX)
            .cmp(&supported_embedding_preset_order(&right.model_id).unwrap_or(usize::MAX))
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.local_model_path.cmp(&right.local_model_path))
    });

    EmbeddingLocalModelCatalog {
        managed_directory: managed_directory.to_string_lossy().to_string(),
        presets,
        available_models,
    }
}

pub fn download_local_model_with_progress<F>(
    model_storage_dir: &Path,
    model_id: &str,
    download_source: &str,
    cancel_requested: &AtomicBool,
    on_progress: &mut F,
) -> Result<(), EmbeddingDownloadError>
where
    F: FnMut(EmbeddingActivationProgress),
{
    let trimmed_model_id = model_id.trim();
    if trimmed_model_id.is_empty() {
        return Err(EmbeddingDownloadError::failed("Local model id is required"));
    }
    let managed_directory = managed_model_root(model_storage_dir);
    if let Some(model) = fastembed_model_for_id(trimmed_model_id) {
        prefetch_fastembed_model_with_cancel(
            &model,
            &managed_directory,
            download_source,
            Some(cancel_requested),
            on_progress,
        )
    } else if let Some(preset) = supported_embedding_preset(trimmed_model_id) {
        match preset.download_kind {
            EmbeddingPresetDownloadKind::Fastembed => Err(EmbeddingDownloadError::failed(format!(
                "Local model '{}' is not supported for automatic download",
                trimmed_model_id
            ))),
            EmbeddingPresetDownloadKind::HuggingFace => {
                download_custom_huggingface_model_with_cancel(
                    preset.id,
                    supported_embedding_preset_download_model_id(preset),
                    &managed_directory,
                    download_source,
                    Some(cancel_requested),
                    on_progress,
                )
            }
        }
    } else {
        download_custom_huggingface_model_with_cancel(
            trimmed_model_id,
            trimmed_model_id,
            &managed_directory,
            download_source,
            Some(cancel_requested),
            on_progress,
        )
    }
}

pub fn inspect_local_model_directory(path: &Path) -> EmbeddingLocalModelDirectoryInspection {
    let normalized_path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let metadata = load_managed_local_model_metadata(&normalized_path);
    let label = metadata
        .as_ref()
        .map(|value| value.label.clone())
        .unwrap_or_else(|| {
            normalized_path
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| !value.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| normalized_path.to_string_lossy().to_string())
        });
    let model_file =
        find_manual_model_file(&normalized_path).map(|value| value.to_string_lossy().to_string());
    let missing_files = required_local_model_files(&normalized_path);

    EmbeddingLocalModelDirectoryInspection {
        path: normalized_path.to_string_lossy().to_string(),
        label,
        ready: model_file.is_some() && missing_files.is_empty(),
        model_file,
        missing_files,
    }
}

fn local_device_route(device_policy: &str) -> String {
    match normalize_device_policy(device_policy) {
        DEVICE_POLICY_GPU_DIRECTML => "directml".to_string(),
        DEVICE_POLICY_GPU_CUDA => "cuda".to_string(),
        _ => "cpu".to_string(),
    }
}

fn configured_local_device_route(device_policy: &str) -> String {
    local_device_route(device_policy)
}

fn configured_local_device_name(device_policy: &str) -> Option<String> {
    match normalize_device_policy(device_policy) {
        DEVICE_POLICY_GPU_DIRECTML | DEVICE_POLICY_GPU_CUDA => preferred_gpu_device_name(),
        _ => current_cpu_device_name(),
    }
}

fn device_policy_display_name(device_policy: &str) -> &'static str {
    match normalize_device_policy(device_policy) {
        DEVICE_POLICY_GPU_DIRECTML => "GPU-DirectML",
        DEVICE_POLICY_GPU_CUDA => "GPU-CUDA",
        _ => "CPU-Fastembed",
    }
}

fn normalize_runtime_error_message(error: &str, fallback: &str) -> String {
    normalize_runtime_error_message_with_debug(error, None, fallback)
}

fn normalize_runtime_error_message_with_debug(
    error: &str,
    debug: Option<&str>,
    fallback: &str,
) -> String {
    let trimmed = error.trim().trim_end_matches(':').trim();
    let debug = debug
        .map(compact_runtime_diagnostic)
        .filter(|value| !value.is_empty());
    if trimmed.is_empty() {
        if let Some(debug) = debug {
            format!("{fallback} [{debug}]")
        } else {
            fallback.to_string()
        }
    } else if let Some(debug) = debug {
        if debug == trimmed {
            trimmed.to_string()
        } else {
            format!("{trimmed} [{debug}]")
        }
    } else {
        trimmed.to_string()
    }
}

fn compact_runtime_diagnostic(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}

#[derive(Debug, Clone)]
struct ExecutionProviderPlan {
    route: &'static str,
    backend_label: &'static str,
    device_name: String,
    providers: Vec<ExecutionProviderDispatch>,
}

#[derive(Debug, Clone)]
struct InitializedLocalDevice {
    route: String,
    device_name: String,
}

#[cfg(windows)]
fn preferred_local_execution_provider_plans(device_policy: &str) -> Vec<ExecutionProviderPlan> {
    match normalize_device_policy(device_policy) {
        DEVICE_POLICY_GPU_DIRECTML => directml_device_ids_to_try()
            .into_iter()
            .map(|device_id| ExecutionProviderPlan {
                route: "directml",
                backend_label: "GPU-DirectML",
                device_name: query_gpu_adapter_name_for_index(device_id)
                    .unwrap_or_else(|| "GPU".to_string()),
                providers: vec![DirectML::default()
                    .with_device_id(device_id)
                    .build()
                    .error_on_failure()],
            })
            .collect(),
        DEVICE_POLICY_GPU_CUDA => vec![ExecutionProviderPlan {
            route: "cuda",
            backend_label: "GPU-CUDA",
            device_name: preferred_gpu_device_name().unwrap_or_else(|| "GPU".to_string()),
            providers: vec![CUDA::default().build().error_on_failure()],
        }],
        _ => Vec::new(),
    }
}

#[cfg(not(windows))]
fn preferred_local_execution_provider_plans(_device_policy: &str) -> Vec<ExecutionProviderPlan> {
    Vec::new()
}

fn initialize_local_embedding_with_device_policy<F>(
    requested_model: &str,
    model_dir: &Path,
    device_policy: &str,
    mut build: F,
) -> Result<(TextEmbedding, InitializedLocalDevice), String>
where
    F: FnMut(Vec<ExecutionProviderDispatch>) -> Result<TextEmbedding, String>,
{
    let normalized_policy = normalize_device_policy(device_policy);
    if normalized_policy == DEVICE_POLICY_CPU_FASTEMBED {
        return Ok((
            build(Vec::new())?,
            InitializedLocalDevice {
                route: "cpu".to_string(),
                device_name: configured_local_device_name(normalized_policy)
                    .unwrap_or_else(|| "CPU".to_string()),
            },
        ));
    }

    let mut attempted_gpu = false;
    let mut last_gpu_error: Option<String> = None;
    for plan in preferred_local_execution_provider_plans(normalized_policy) {
        attempted_gpu = true;
        match build(plan.providers) {
            Ok(embedding) => {
                return Ok((
                    embedding,
                    InitializedLocalDevice {
                        route: plan.route.to_string(),
                        device_name: plan.device_name,
                    },
                ));
            }
            Err(error) => {
                let message = format!(
                    "Failed to initialize {} backend for local embeddings: {}. Switch to CPU-Fastembed or another backend.",
                    plan.backend_label,
                    normalize_runtime_error_message(&error, "Unknown runtime error"),
                );
                tracing::error!(
                    log_module = "knowledge_index",
                    requested_model = %requested_model,
                    device_policy = %normalized_policy,
                    backend = %plan.backend_label,
                    diagnostics = %gpu_backend_init_diagnostics(
                        requested_model,
                        model_dir,
                        normalized_policy,
                        plan.route,
                    ),
                    "{message}"
                );
                last_gpu_error = Some(message);
            }
        }
    }

    if !attempted_gpu {
        let message = format!(
            "{} backend is unavailable in this build. Switch to CPU-Fastembed.",
            device_policy_display_name(normalized_policy)
        );
        tracing::error!(
            log_module = "knowledge_index",
            requested_model = %requested_model,
            device_policy = %normalized_policy,
            diagnostics = %gpu_backend_init_diagnostics(
                requested_model,
                model_dir,
                normalized_policy,
                "unavailable",
            ),
            "{message}"
        );
        return Err(message);
    }

    Err(last_gpu_error.unwrap_or_else(|| {
        format!(
            "Failed to initialize {} backend for local embeddings. Switch to CPU-Fastembed.",
            device_policy_display_name(normalized_policy)
        )
    }))
}

fn gpu_backend_init_diagnostics(
    requested_model: &str,
    model_dir: &Path,
    device_policy: &str,
    backend_route: &str,
) -> String {
    let diagnostic_model_dir = runtime_diagnostic_model_dir(requested_model, model_dir);
    let model_file = find_manual_model_file(&diagnostic_model_dir)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "missing".to_string());
    let tokenizer_files = LOCAL_TOKENIZER_FILE_NAMES
        .iter()
        .map(|name| {
            format!(
                "{name}={}",
                find_named_file(&diagnostic_model_dir, name).is_some()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut parts = vec![
        format!("requested_model={requested_model}"),
        format!("device_policy={device_policy}"),
        format!("backend_route={backend_route}"),
        format!("managed_model_dir={}", model_dir.display()),
        format!("diagnostic_model_dir={}", diagnostic_model_dir.display()),
        format!("model_file={model_file}"),
        format!(
            "cache_dir={}",
            effective_fastembed_cache_dir(model_dir).display()
        ),
        format!("tokenizer_files={tokenizer_files}"),
    ];

    #[cfg(windows)]
    {
        parts.push(format!(
            "directml_available={}",
            execution_provider_availability(DirectML::default().is_available())
        ));
        parts.push(format!(
            "cuda_available={}",
            execution_provider_availability(CUDA::default().is_available())
        ));
        parts.push(format!("dxgi={}", summarize_dxgi_adapters()));
    }

    parts.join("; ")
}

fn runtime_diagnostic_model_dir(requested_model: &str, model_dir: &Path) -> PathBuf {
    let requested_model = requested_model.trim();
    if let Some(model) = fastembed_model_for_id(requested_model) {
        let cache_root = effective_fastembed_cache_dir(model_dir);
        if let Some(cache_dir) = fastembed_model_cache_directory(&cache_root, &model) {
            return cache_dir;
        }
    }
    model_dir.to_path_buf()
}

#[cfg(windows)]
fn execution_provider_availability(result: Result<bool, ort::Error>) -> String {
    match result {
        Ok(value) => value.to_string(),
        Err(error) => normalize_runtime_error_message_with_debug(
            &error.to_string(),
            Some(&format!("{error:#?}")),
            "query_failed",
        ),
    }
}

#[cfg(windows)]
fn summarize_dxgi_adapters() -> String {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory6, DXGI_ADAPTER_DESC1,
        DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
    };

    fn adapter_name(desc: &DXGI_ADAPTER_DESC1) -> String {
        let end = desc
            .Description
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(desc.Description.len());
        String::from_utf16_lossy(&desc.Description[..end])
    }

    let factory: IDXGIFactory6 = match unsafe { CreateDXGIFactory1() } {
        Ok(factory) => factory,
        Err(error) => {
            return format!(
                "factory_error={}",
                normalize_runtime_error_message_with_debug(
                    &error.to_string(),
                    Some(&format!("{error:#?}")),
                    "CreateDXGIFactory1 failed"
                )
            );
        }
    };

    let preferred =
        unsafe { factory.EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE) }
            .ok()
            .and_then(|adapter: IDXGIAdapter1| unsafe { adapter.GetDesc1().ok() })
            .map(|desc| {
                format!(
                    "preferred={}({} MB,vendor=0x{:04x},device=0x{:04x})",
                    adapter_name(&desc),
                    desc.DedicatedVideoMemory / 1024 / 1024,
                    desc.VendorId,
                    desc.DeviceId
                )
            });

    let mut adapters = Vec::new();
    let mut index = 0;
    loop {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(_) => break,
        };
        let desc = match unsafe { adapter.GetDesc1() } {
            Ok(desc) => desc,
            Err(error) => {
                adapters.push(format!(
                    "#{index}:desc_error={}",
                    normalize_runtime_error_message_with_debug(
                        &error.to_string(),
                        Some(&format!("{error:#?}")),
                        "GetDesc1 failed"
                    )
                ));
                index += 1;
                continue;
            }
        };
        adapters.push(format!(
            "#{index}:{}({} MB,vendor=0x{:04x},device=0x{:04x})",
            adapter_name(&desc),
            desc.DedicatedVideoMemory / 1024 / 1024,
            desc.VendorId,
            desc.DeviceId
        ));
        index += 1;
    }

    if let Some(preferred) = preferred {
        if adapters.is_empty() {
            preferred
        } else {
            format!("{preferred}; adapters={}", adapters.join(" | "))
        }
    } else if adapters.is_empty() {
        "no_adapters".to_string()
    } else {
        format!("adapters={}", adapters.join(" | "))
    }
}

fn probe_embedding_dimension(embedding: &mut TextEmbedding) -> Result<usize, String> {
    embedding
        .embed(["dimension probe"], Some(1))
        .map_err(|e| format!("Failed to probe embedding dimension: {}", e))
        .and_then(|vectors| {
            vectors
                .first()
                .map(Vec::len)
                .filter(|dim| *dim > 0)
                .ok_or_else(|| "Embedding runtime returned an empty vector".to_string())
        })
}

struct AggregatedDownloadProgress<'a, F>
where
    F: FnMut(EmbeddingActivationProgress),
{
    file_name: String,
    overall_total_bytes: u64,
    completed_bytes: &'a mut u64,
    current_file_total_bytes: u64,
    current_file_downloaded_bytes: u64,
    on_progress: &'a mut F,
}

impl<'a, F> AggregatedDownloadProgress<'a, F>
where
    F: FnMut(EmbeddingActivationProgress),
{
    fn new(
        file_name: &str,
        overall_total_bytes: u64,
        completed_bytes: &'a mut u64,
        on_progress: &'a mut F,
    ) -> Self {
        Self {
            file_name: file_name.to_string(),
            overall_total_bytes,
            completed_bytes,
            current_file_total_bytes: 0,
            current_file_downloaded_bytes: 0,
            on_progress,
        }
    }

    fn emit_progress(&mut self) {
        let total_bytes = self
            .overall_total_bytes
            .max((*self.completed_bytes).saturating_add(self.current_file_total_bytes));
        let downloaded_bytes =
            (*self.completed_bytes).saturating_add(self.current_file_downloaded_bytes);
        let progress = if total_bytes == 0 {
            1.0
        } else {
            (downloaded_bytes as f64 / total_bytes as f64).clamp(0.0, 1.0)
        };
        (self.on_progress)(EmbeddingActivationProgress::Download {
            file_name: self.file_name.clone(),
            downloaded_bytes,
            total_bytes,
            progress,
        });
    }
    fn start_file(&mut self, filename: &str, size: u64) {
        self.file_name = filename.to_string();
        self.current_file_total_bytes = size;
        self.current_file_downloaded_bytes = 0;
        self.emit_progress();
    }

    fn advance(&mut self, size: u64) {
        self.current_file_downloaded_bytes =
            self.current_file_downloaded_bytes.saturating_add(size);
        self.emit_progress();
    }

    fn finish_file(&mut self) {
        let completed_file_bytes = self
            .current_file_total_bytes
            .max(self.current_file_downloaded_bytes);
        self.current_file_downloaded_bytes = completed_file_bytes;
        self.emit_progress();
        *self.completed_bytes = (*self.completed_bytes)
            .saturating_add(completed_file_bytes)
            .min(self.overall_total_bytes.max(completed_file_bytes));
    }
}

fn manual_model_files_ready(model_dir: &Path) -> bool {
    find_manual_model_file(model_dir).is_some()
        && REQUIRED_LOCAL_TOKENIZER_FILE_NAMES
            .iter()
            .all(|file_name| find_named_file(model_dir, file_name).is_some())
}

fn required_local_model_files(model_dir: &Path) -> Vec<String> {
    let mut missing = Vec::new();
    if find_manual_model_file(model_dir).is_none() {
        missing.push("model.onnx".to_string());
    }
    for file_name in REQUIRED_LOCAL_TOKENIZER_FILE_NAMES {
        if find_named_file(model_dir, file_name).is_none() {
            missing.push(file_name.to_string());
        }
    }
    missing
}

fn find_manual_model_file(model_dir: &Path) -> Option<PathBuf> {
    for relative_path in LOCAL_MODEL_FILE_CANDIDATES {
        let candidate = join_relative_path(model_dir, relative_path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    WalkDir::new(model_dir)
        .follow_links(true)
        .max_depth(4)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.path().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("onnx"))
                    .unwrap_or(false)
        })
        .map(|entry| entry.into_path())
}

fn load_manual_tokenizer_files(model_dir: &Path) -> Result<TokenizerFiles, String> {
    Ok(TokenizerFiles {
        tokenizer_file: read_required_named_file(model_dir, "tokenizer.json")?,
        config_file: read_required_named_file(model_dir, "config.json")?,
        special_tokens_map_file: read_optional_named_file(
            model_dir,
            "special_tokens_map.json",
            EMPTY_JSON_OBJECT,
        )?,
        tokenizer_config_file: read_required_named_file(model_dir, "tokenizer_config.json")?,
    })
}

fn read_required_named_file(model_dir: &Path, file_name: &str) -> Result<Vec<u8>, String> {
    let path = find_named_file(model_dir, file_name).ok_or_else(|| {
        format!(
            "Manual local model is missing required file '{}' in {}",
            file_name,
            model_dir.display()
        )
    })?;
    std::fs::read(&path).map_err(|e| format!("Failed to read '{}': {}", path.display(), e))
}

fn read_optional_named_file(
    model_dir: &Path,
    file_name: &str,
    default_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let Some(path) = find_named_file(model_dir, file_name) else {
        return Ok(default_bytes.to_vec());
    };
    std::fs::read(&path).map_err(|e| format!("Failed to read '{}': {}", path.display(), e))
}

fn find_named_file(model_dir: &Path, file_name: &str) -> Option<PathBuf> {
    WalkDir::new(model_dir)
        .follow_links(true)
        .max_depth(5)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.path().is_file()
                && entry
                    .file_name()
                    .to_str()
                    .map(|name| name.eq_ignore_ascii_case(file_name))
                    .unwrap_or(false)
        })
        .map(|entry| entry.into_path())
}

fn collect_manual_external_initializers(
    model_file: &Path,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let Some(parent_dir) = model_file.parent() else {
        return Ok(Vec::new());
    };

    let mut files = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(parent_dir)
        .map_err(|e| format!("Failed to read model dir '{}': {}", parent_dir.display(), e))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if !path.is_file() || path == model_file {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if should_skip_manual_initializer(file_name) {
            continue;
        }
        let buffer = std::fs::read(&path).map_err(|e| {
            format!(
                "Failed to read external initializer '{}': {}",
                path.display(),
                e
            )
        })?;
        files.push((file_name.to_string(), buffer));
    }
    Ok(files)
}

fn should_skip_manual_initializer(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    LOCAL_TOKENIZER_FILE_NAMES
        .iter()
        .any(|name| lower == name.to_ascii_lowercase())
        || lower == "modules.json"
        || lower == "config_sentence_transformers.json"
        || lower == "sentence_bert_config.json"
        || lower == ".gitattributes"
        || lower.ends_with(".json")
        || lower.ends_with(".txt")
        || lower.ends_with(".md")
        || lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
}

fn load_managed_local_model_metadata(model_dir: &Path) -> Option<ManagedLocalModelMetadata> {
    let metadata_path = model_dir.join(LOCAL_MODEL_METADATA_FILE_NAME);
    let raw = std::fs::read_to_string(metadata_path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_managed_local_model_metadata(model_dir: &Path, model_id: &str) -> Result<(), String> {
    let label = supported_embedding_preset(model_id)
        .map(|preset| preset.label)
        .unwrap_or(model_id);
    let metadata = ManagedLocalModelMetadata {
        model_id: model_id.to_string(),
        label: label.to_string(),
        pooling: manual_pooling_override_for_model(model_id)
            .map(|pooling| pooling.as_metadata_str().to_string()),
    };
    let metadata_path = model_dir.join(LOCAL_MODEL_METADATA_FILE_NAME);
    let raw = serde_json::to_string_pretty(&metadata)
        .map_err(|e| format!("Failed to serialize local model metadata: {}", e))?;
    std::fs::write(&metadata_path, raw).map_err(|e| {
        format!(
            "Failed to write local model metadata '{}': {}",
            metadata_path.display(),
            e
        )
    })
}

fn is_hf_cache_directory_name(dir_name: &str) -> bool {
    dir_name.starts_with("models--")
        || dir_name.starts_with("datasets--")
        || dir_name.starts_with("spaces--")
}

fn manual_pooling_override_for_model(model_id: &str) -> Option<ManualPoolingMode> {
    let normalized = model_id.trim().to_ascii_lowercase();
    if normalized.contains("qwen3-embedding") {
        return Some(ManualPoolingMode::LastToken);
    }
    None
}

fn resolve_manual_pooling(
    model_dir: &Path,
    requested_model: &str,
    known_model: Option<&FastembedModel>,
) -> Result<Option<ManualPoolingMode>, String> {
    if let Some(model) = known_model {
        return Ok(
            TextEmbedding::get_default_pooling_method(model).map(manual_pooling_from_fastembed)
        );
    }

    if let Some(metadata_pooling) = load_managed_local_model_metadata(model_dir)
        .and_then(|metadata| metadata.pooling)
        .and_then(|pooling| manual_pooling_from_metadata_str(&pooling))
    {
        return Ok(Some(metadata_pooling));
    }

    for relative_path in [
        "1_Pooling/config.json",
        "sentence_bert_config.json",
        "config_sentence_transformers.json",
    ] {
        if let Some(pooling) = load_pooling_mode_from_json_file(model_dir, relative_path)? {
            return Ok(Some(pooling));
        }
    }

    Ok(manual_pooling_override_for_model(requested_model))
}

fn load_manual_pooling(
    model_dir: &Path,
    requested_model: &str,
) -> Result<Option<ManualPoolingMode>, String> {
    resolve_manual_pooling(model_dir, requested_model, None)
}

fn load_pooling_mode_from_json_file(
    model_dir: &Path,
    relative_path: &str,
) -> Result<Option<ManualPoolingMode>, String> {
    let config_path = join_relative_path(model_dir, relative_path);
    if !config_path.is_file() {
        return Ok(None);
    }

    let data = std::fs::read_to_string(&config_path).map_err(|e| {
        format!(
            "Failed to read pooling config '{}': {}",
            config_path.display(),
            e
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&data).map_err(|e| {
        format!(
            "Failed to parse pooling config '{}': {}",
            config_path.display(),
            e
        )
    })?;

    if let Some(pooling) = parse_pooling_mode_value(&value) {
        return Ok(Some(pooling));
    }

    Ok(None)
}

fn parse_pooling_mode_value(value: &serde_json::Value) -> Option<ManualPoolingMode> {
    if value["pooling_mode_lasttoken"].as_bool().unwrap_or(false) {
        return Some(ManualPoolingMode::LastToken);
    }
    if value["pooling_mode_cls_token"].as_bool().unwrap_or(false) {
        return Some(ManualPoolingMode::Cls);
    }
    if value["pooling_mode_mean_tokens"].as_bool().unwrap_or(false)
        || value["pooling_mode_mean_sqrt_len_tokens"]
            .as_bool()
            .unwrap_or(false)
    {
        return Some(ManualPoolingMode::Mean);
    }
    value["pooling_mode"]
        .as_str()
        .and_then(manual_pooling_from_metadata_str)
}

fn join_relative_path(root: &Path, relative_path: &str) -> PathBuf {
    relative_path
        .split('/')
        .fold(root.to_path_buf(), |mut path, component| {
            path.push(component);
            path
        })
}

fn path_ends_with_components(path: &Path, relative_path: &str) -> bool {
    let relative_components: Vec<_> = relative_path
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    let path_components: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect();

    if path_components.len() < relative_components.len() {
        return false;
    }

    let start = path_components.len() - relative_components.len();
    path_components[start..]
        .iter()
        .zip(relative_components.iter())
        .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

#[cfg(test)]
mod tests {
    use super::{
        build_remote_repo_tree_url, build_supported_ureq_proxy_url, config_path, extract_next_link,
        load_config, local_model_catalog, managed_model_root, migrate_legacy_managed_model_root,
        normalize_device_policy, prepare_local_model_download_network,
        prioritize_directml_adapter_indices, resolve_next_link_url, sanitize_model_id,
        sanitize_proxy_display_url, save_config, select_local_model_source,
        supported_embedding_preset, supported_embedding_preset_download_model_id,
        DirectMlAdapterCandidate, EmbeddingConfig, EmbeddingManager, FastembedModel,
        LocalModelRoute, ManualPoolingMode, TextEmbedding, DEVICE_POLICY_CPU_FASTEMBED,
        DEVICE_POLICY_GPU_CUDA, DEVICE_POLICY_GPU_DIRECTML, LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR,
        LOCAL_MODEL_DOWNLOAD_SOURCE_OFFICIAL,
    };
    use http::Uri;
    use tempfile::tempdir;

    struct TempEnvGuard {
        _lock_guard: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<String>)>,
    }

    impl TempEnvGuard {
        fn set(vars: &[(&str, Option<&str>)]) -> Self {
            static ENV_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
            let lock_guard = ENV_LOCK
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .expect("env lock");
            let mut saved = Vec::with_capacity(vars.len());
            for (key, value) in vars {
                saved.push((key.to_string(), std::env::var(key).ok()));
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            Self {
                _lock_guard: lock_guard,
                saved,
            }
        }
    }

    impl Drop for TempEnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..).rev() {
                match value {
                    Some(value) => std::env::set_var(&key, value),
                    None => std::env::remove_var(&key),
                }
            }
        }
    }

    fn create_dummy_manual_model_files(model_dir: &std::path::Path) {
        std::fs::create_dir_all(model_dir).expect("create model dir");
        std::fs::write(model_dir.join("model.onnx"), b"dummy").expect("write model");
        std::fs::write(model_dir.join("tokenizer.json"), b"{}").expect("write tokenizer");
        std::fs::write(model_dir.join("config.json"), b"{}").expect("write config");
        std::fs::write(model_dir.join("special_tokens_map.json"), b"{}")
            .expect("write special tokens");
        std::fs::write(
            model_dir.join("tokenizer_config.json"),
            br#"{"model_max_length":512,"pad_token":"[PAD]"}"#,
        )
        .expect("write tokenizer config");
    }

    fn create_dummy_manual_model_files_without_special_tokens_map(model_dir: &std::path::Path) {
        std::fs::create_dir_all(model_dir).expect("create model dir");
        std::fs::write(model_dir.join("model.onnx"), b"dummy").expect("write model");
        std::fs::write(model_dir.join("tokenizer.json"), b"{}").expect("write tokenizer");
        std::fs::write(model_dir.join("config.json"), b"{}").expect("write config");
        std::fs::write(
            model_dir.join("tokenizer_config.json"),
            br#"{"model_max_length":512,"pad_token":"[PAD]"}"#,
        )
        .expect("write tokenizer config");
    }

    fn create_dummy_fastembed_cache(model_storage_dir: &std::path::Path, model: FastembedModel) {
        let info = TextEmbedding::get_model_info(&model).expect("model info");
        let revision = "test-revision";
        let repo_dir = managed_model_root(model_storage_dir)
            .join(format!("models--{}", info.model_code.replace('/', "--")));
        let snapshot_dir = repo_dir.join("snapshots").join(revision);
        std::fs::create_dir_all(&snapshot_dir).expect("create snapshot dir");
        std::fs::create_dir_all(repo_dir.join("refs")).expect("create refs dir");
        std::fs::write(repo_dir.join("refs").join("main"), revision).expect("write revision");

        let mut required_files = vec![
            info.model_file.clone(),
            "tokenizer.json".to_string(),
            "config.json".to_string(),
            "special_tokens_map.json".to_string(),
            "tokenizer_config.json".to_string(),
        ];
        required_files.extend(info.additional_files.clone());

        for relative_path in required_files {
            let target = snapshot_dir.join(relative_path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).expect("create fastembed parent");
            }
            std::fs::write(target, b"test").expect("write fastembed file");
        }
    }

    #[test]
    fn normalize_device_policy_accepts_current_values() {
        assert_eq!(
            normalize_device_policy(DEVICE_POLICY_CPU_FASTEMBED),
            DEVICE_POLICY_CPU_FASTEMBED
        );
        assert_eq!(
            normalize_device_policy(DEVICE_POLICY_GPU_DIRECTML),
            DEVICE_POLICY_GPU_DIRECTML
        );
        assert_eq!(
            normalize_device_policy(DEVICE_POLICY_GPU_CUDA),
            DEVICE_POLICY_GPU_CUDA
        );
    }

    #[test]
    fn normalize_device_policy_falls_back_to_cpu_for_unknown_values() {
        assert_eq!(normalize_device_policy(""), DEVICE_POLICY_CPU_FASTEMBED);
        assert_eq!(normalize_device_policy("cpu"), DEVICE_POLICY_CPU_FASTEMBED);
        #[cfg(windows)]
        assert_eq!(normalize_device_policy("gpu"), DEVICE_POLICY_GPU_DIRECTML);
        #[cfg(not(windows))]
        assert_eq!(normalize_device_policy("gpu"), DEVICE_POLICY_CPU_FASTEMBED);
        assert_eq!(
            normalize_device_policy("unknown-backend"),
            DEVICE_POLICY_CPU_FASTEMBED
        );
    }

    #[test]
    fn build_remote_repo_tree_url_encodes_model_repo_path() {
        let url = build_remote_repo_tree_url(
            "https://hf-mirror.com",
            "onnx-community/Qwen3-Embedding-4B-ONNX",
            "main",
        )
        .expect("tree url");
        assert_eq!(
            url,
            "https://hf-mirror.com/api/models/onnx-community/Qwen3-Embedding-4B-ONNX/tree/main?recursive=1"
        );
    }

    #[test]
    fn extract_next_link_returns_next_page_url() {
        let link = extract_next_link(
            r#"<https://hf-mirror.com/api/models/test/tree/main?recursive=1&cursor=abc>; rel="next", <https://hf-mirror.com/api/models/test/tree/main?recursive=1&cursor=def>; rel="prev""#,
        );
        assert_eq!(
            link,
            Some("https://hf-mirror.com/api/models/test/tree/main?recursive=1&cursor=abc")
        );
    }

    #[test]
    fn resolve_next_link_url_supports_relative_links() {
        let url = resolve_next_link_url(
            "https://hf-mirror.com",
            "/api/models/test/tree/main?recursive=1&cursor=abc",
        )
        .expect("resolved next link");
        assert_eq!(
            url,
            "https://hf-mirror.com/api/models/test/tree/main?recursive=1&cursor=abc"
        );
    }

    #[test]
    fn load_config_falls_back_to_cpu_for_unknown_device_policy_values() {
        let dir = tempdir().expect("temp dir");
        std::fs::write(
            config_path(dir.path()),
            r#"{"enabled":true,"embeddingMode":"local","devicePolicy":"unknown-backend"}"#,
        )
        .expect("write config");

        let config = load_config(dir.path());
        assert_eq!(config.device_policy, DEVICE_POLICY_CPU_FASTEMBED);
    }

    #[test]
    fn save_config_persists_current_device_policy_values() {
        let dir = tempdir().expect("temp dir");
        let config = EmbeddingConfig {
            device_policy: DEVICE_POLICY_CPU_FASTEMBED.to_string(),
            ..EmbeddingConfig::default()
        };

        save_config(dir.path(), &config).expect("save config");
        let saved = load_config(dir.path());
        assert_eq!(saved.device_policy, DEVICE_POLICY_CPU_FASTEMBED);
    }

    #[test]
    fn default_config_uses_official_local_model_download_source() {
        let config = EmbeddingConfig::default();
        assert_eq!(
            config.local_model_download_source,
            LOCAL_MODEL_DOWNLOAD_SOURCE_OFFICIAL
        );
    }

    #[test]
    fn load_config_normalizes_local_model_download_source() {
        let dir = tempdir().expect("temp dir");
        std::fs::write(
            config_path(dir.path()),
            r#"{"enabled":true,"embeddingMode":"local","localModelDownloadSource":"mirror"}"#,
        )
        .expect("write config");

        let config = load_config(dir.path());
        assert_eq!(
            config.local_model_download_source,
            LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR
        );
    }

    #[test]
    fn save_config_persists_local_model_download_source() {
        let dir = tempdir().expect("temp dir");
        let config = EmbeddingConfig {
            local_model_download_source: LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR.to_string(),
            ..EmbeddingConfig::default()
        };

        save_config(dir.path(), &config).expect("save config");
        let saved = load_config(dir.path());
        assert_eq!(
            saved.local_model_download_source,
            LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR
        );
    }

    #[test]
    fn select_local_model_source_prefers_preset_without_explicit_manual_path() {
        let dir = tempdir().expect("temp dir");
        let config = EmbeddingConfig::default();
        let manual_dir = dir
            .path()
            .join(sanitize_model_id("Qwen/Qwen3-Embedding-4B"));
        create_dummy_manual_model_files(&manual_dir);

        let selection = select_local_model_source(&config, dir.path()).expect("select source");
        assert_eq!(selection.route, LocalModelRoute::Preset);
    }

    #[test]
    fn select_local_model_source_uses_manual_when_explicit_directory_is_set() {
        let dir = tempdir().expect("temp dir");
        let manual_dir = dir.path().join("manual-model");
        create_dummy_manual_model_files(&manual_dir);
        let config = EmbeddingConfig {
            local_model_path: manual_dir.display().to_string(),
            ..EmbeddingConfig::default()
        };

        let selection = select_local_model_source(&config, dir.path()).expect("select source");
        assert_eq!(selection.route, LocalModelRoute::Manual);
    }

    #[test]
    fn embedding_manager_download_state_follows_fastembed_preset_route_priority() {
        let dir = tempdir().expect("temp dir");
        let manual_dir =
            managed_model_root(dir.path()).join(sanitize_model_id("BAAI/bge-small-zh-v1.5"));
        create_dummy_manual_model_files(&manual_dir);

        let manager = EmbeddingManager::new(
            EmbeddingConfig {
                local_model: "BAAI/bge-small-zh-v1.5".to_string(),
                ..EmbeddingConfig::default()
            },
            dir.path(),
        );
        assert!(!manager.is_model_downloaded());
    }

    #[test]
    fn embedding_manager_download_state_uses_managed_manual_files_for_repo_presets() {
        let dir = tempdir().expect("temp dir");
        let manual_dir =
            managed_model_root(dir.path()).join(sanitize_model_id("Qwen/Qwen3-Embedding-4B"));
        create_dummy_manual_model_files(&manual_dir);

        let manager = EmbeddingManager::new(EmbeddingConfig::default(), dir.path());
        assert!(manager.is_model_downloaded());
    }

    #[test]
    fn activation_requires_downloaded_huggingface_preset() {
        let dir = tempdir().expect("temp dir");
        let mut manager = EmbeddingManager::new(
            EmbeddingConfig {
                enabled: true,
                local_model: "Qwen/Qwen3-Embedding-0.6B".to_string(),
                ..EmbeddingConfig::default()
            },
            dir.path(),
        );

        let err = manager
            .activate_with_progress(&mut |_| {})
            .expect_err("activation should require a manual model download");

        assert!(err.contains("has not been downloaded"));
        assert!(!managed_model_root(dir.path())
            .join(sanitize_model_id("Qwen/Qwen3-Embedding-0.6B"))
            .exists());
    }

    #[test]
    fn activation_requires_cached_fastembed_preset() {
        let dir = tempdir().expect("temp dir");
        let hf_home = dir.path().join("hf-home");
        let hf_home_string = hf_home.to_string_lossy().to_string();
        let _env = TempEnvGuard::set(&[("HF_HOME", Some(hf_home_string.as_str()))]);
        let mut manager = EmbeddingManager::new(
            EmbeddingConfig {
                enabled: true,
                local_model: "BAAI/bge-small-zh-v1.5".to_string(),
                ..EmbeddingConfig::default()
            },
            dir.path(),
        );

        let err = manager
            .activate_with_progress(&mut |_| {})
            .expect_err("activation should require a cached fastembed model");

        assert!(err.contains("has not been downloaded"));
        assert!(!hf_home.exists());
    }

    #[test]
    fn inspect_local_model_directory_allows_missing_special_tokens_map() {
        let dir = tempdir().expect("temp dir");
        create_dummy_manual_model_files_without_special_tokens_map(dir.path());

        let inspection = super::inspect_local_model_directory(dir.path());
        assert!(inspection.ready);
        assert!(inspection.missing_files.is_empty());
    }

    #[test]
    fn load_manual_tokenizer_files_defaults_missing_special_tokens_map() {
        let dir = tempdir().expect("temp dir");
        create_dummy_manual_model_files_without_special_tokens_map(dir.path());

        let tokenizer_files =
            super::load_manual_tokenizer_files(dir.path()).expect("load tokenizer files");
        assert_eq!(tokenizer_files.special_tokens_map_file, b"{}");
    }

    #[test]
    fn local_model_catalog_surfaces_manual_models_without_special_tokens_map() {
        let dir = tempdir().expect("temp dir");
        let manual_dir = managed_model_root(dir.path()).join("manual-model");
        create_dummy_manual_model_files_without_special_tokens_map(&manual_dir);

        let catalog = local_model_catalog(dir.path());
        assert!(catalog.available_models.iter().any(|model| {
            model.label == "manual-model"
                && model.local_model_path == manual_dir.display().to_string()
        }));
    }

    #[test]
    fn local_model_catalog_surfaces_cached_fastembed_models() {
        let dir = tempdir().expect("temp dir");
        create_dummy_fastembed_cache(dir.path(), FastembedModel::BGESmallZHV15);

        let catalog = local_model_catalog(dir.path());
        assert!(catalog.available_models.iter().any(|model| {
            model.model_id == "BAAI/bge-small-zh-v1.5" && model.local_model_path.is_empty()
        }));
    }

    #[test]
    fn qwen_presets_download_from_onnx_repos() {
        let qwen_small =
            supported_embedding_preset("Qwen/Qwen3-Embedding-0.6B").expect("qwen 0.6 preset");
        let qwen_large =
            supported_embedding_preset("Qwen/Qwen3-Embedding-4B").expect("qwen 4b preset");

        assert_eq!(
            supported_embedding_preset_download_model_id(qwen_small),
            "onnx-community/Qwen3-Embedding-0.6B-ONNX"
        );
        assert_eq!(
            supported_embedding_preset_download_model_id(qwen_large),
            "onnx-community/Qwen3-Embedding-4B-ONNX"
        );
    }

    #[test]
    fn load_manual_pooling_supports_lasttoken() {
        let dir = tempdir().expect("temp dir");
        let pooling_dir = dir.path().join("1_Pooling");
        std::fs::create_dir_all(&pooling_dir).expect("create pooling dir");
        std::fs::write(
            pooling_dir.join("config.json"),
            r#"{
  "pooling_mode_cls_token": false,
  "pooling_mode_mean_tokens": false,
  "pooling_mode_lasttoken": true
}"#,
        )
        .expect("write pooling config");

        let pooling =
            super::load_manual_pooling(dir.path(), "manual-test-model").expect("load pooling");
        assert_eq!(pooling, Some(ManualPoolingMode::LastToken));
    }

    #[test]
    fn load_manual_pooling_uses_qwen_override_without_pooling_config() {
        let dir = tempdir().expect("temp dir");
        let pooling = super::load_manual_pooling(dir.path(), "Qwen/Qwen3-Embedding-0.6B")
            .expect("load pooling");
        assert_eq!(pooling, Some(ManualPoolingMode::LastToken));
    }

    #[cfg(windows)]
    #[test]
    fn build_position_ids_array_handles_left_padding() {
        let attention_mask = ndarray::arr2(&[[0_i64, 0, 1, 1], [1_i64, 1, 1, 0]]);
        let position_ids = super::build_position_ids_array(&attention_mask);
        assert_eq!(
            position_ids,
            ndarray::arr2(&[[0_i64, 0, 0, 1], [0_i64, 1, 2, 0]])
        );
    }

    #[cfg(windows)]
    #[test]
    fn detect_directml_input_schema_recognizes_qwen_cache_inputs() {
        use ort::{
            tensor::{Shape, SymbolicDimensions, TensorElementType},
            value::{Outlet, ValueType},
        };

        let inputs = vec![
            Outlet::new(
                "input_ids",
                ValueType::Tensor {
                    ty: TensorElementType::Int64,
                    shape: Shape::new([-1, -1]),
                    dimension_symbols: SymbolicDimensions::empty(2),
                },
            ),
            Outlet::new(
                "attention_mask",
                ValueType::Tensor {
                    ty: TensorElementType::Int64,
                    shape: Shape::new([-1, -1]),
                    dimension_symbols: SymbolicDimensions::empty(2),
                },
            ),
            Outlet::new(
                "position_ids",
                ValueType::Tensor {
                    ty: TensorElementType::Int64,
                    shape: Shape::new([-1, -1]),
                    dimension_symbols: SymbolicDimensions::empty(2),
                },
            ),
            Outlet::new(
                "past_key_values.0.key",
                ValueType::Tensor {
                    ty: TensorElementType::Float32,
                    shape: Shape::new([-1, 8, -1, 128]),
                    dimension_symbols: SymbolicDimensions::empty(4),
                },
            ),
            Outlet::new(
                "past_key_values.0.value",
                ValueType::Tensor {
                    ty: TensorElementType::Float32,
                    shape: Shape::new([-1, 8, -1, 128]),
                    dimension_symbols: SymbolicDimensions::empty(4),
                },
            ),
        ];

        let schema = super::detect_directml_input_schema(
            &inputs,
            &super::ModelConfigHints {
                num_attention_heads: Some(16),
                num_key_value_heads: Some(8),
                head_dim: Some(128),
            },
        )
        .expect("detect schema");

        assert_eq!(schema.input_ids_name, "input_ids");
        assert_eq!(
            schema.attention_mask_name.as_deref(),
            Some("attention_mask")
        );
        assert_eq!(schema.position_ids_name.as_deref(), Some("position_ids"));
        assert_eq!(schema.token_type_ids_name, None);
        assert_eq!(schema.past_key_values.len(), 1);
        assert_eq!(schema.past_key_values[0].key_name, "past_key_values.0.key");
        assert_eq!(
            schema.past_key_values[0].value_name,
            "past_key_values.0.value"
        );
        assert_eq!(schema.past_key_values[0].num_heads, 8);
        assert_eq!(schema.past_key_values[0].head_dim, 128);
        assert_eq!(schema.past_key_values[0].rank, 4);
    }

    #[cfg(windows)]
    #[test]
    fn directml_output_precedence_prefers_sentence_embedding_before_hidden_state() {
        let sentence_embedding_index = super::DIRECTML_OUTPUT_PRECEDENCE
            .iter()
            .position(|key| matches!(key, fastembed::OutputKey::ByName("sentence_embedding")))
            .expect("sentence embedding output");
        let last_hidden_state_index = super::DIRECTML_OUTPUT_PRECEDENCE
            .iter()
            .position(|key| matches!(key, fastembed::OutputKey::ByName("last_hidden_state")))
            .expect("last hidden state output");

        assert!(sentence_embedding_index < last_hidden_state_index);
    }

    #[test]
    fn build_supported_ureq_proxy_url_maps_http_proxy() {
        let proxy_uri: Uri = "http://127.0.0.1:7890".parse().expect("proxy uri");
        let proxy_url =
            build_supported_ureq_proxy_url(&proxy_uri, Some(("user", "pass"))).expect("proxy");
        assert_eq!(
            proxy_url.as_deref(),
            Some("http://user:pass@127.0.0.1:7890")
        );
    }

    #[test]
    fn build_supported_ureq_proxy_url_skips_https_proxy_scheme() {
        let proxy_uri: Uri = "https://127.0.0.1:8443".parse().expect("proxy uri");
        let proxy_url = build_supported_ureq_proxy_url(&proxy_uri, None).expect("proxy");
        assert_eq!(proxy_url, None);
    }

    #[test]
    fn sanitize_proxy_display_url_hides_credentials() {
        let display = sanitize_proxy_display_url("http://user:pass@127.0.0.1:7890");
        assert_eq!(display, "http://127.0.0.1:7890");
    }

    #[test]
    fn prepare_local_model_download_network_reports_configured_proxy_env() {
        let _guard = TempEnvGuard::set(&[
            ("https_proxy", None),
            ("ALL_PROXY", None),
            ("all_proxy", None),
            ("HTTP_PROXY", None),
            ("http_proxy", None),
            ("HTTPS_PROXY", Some("socks5://user:pass@127.0.0.1:7890")),
        ]);

        let network = prepare_local_model_download_network(LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR)
            .expect("network status");

        assert_eq!(network.source, LOCAL_MODEL_DOWNLOAD_SOURCE_HF_MIRROR);
        assert_eq!(network.endpoint, "https://hf-mirror.com");
        assert_eq!(network.proxy_state, "environment");
        assert_eq!(network.proxy_env_key.as_deref(), Some("HTTPS_PROXY"));
        assert_eq!(
            network.proxy_url.as_deref(),
            Some("socks5://127.0.0.1:7890")
        );
    }

    #[test]
    fn migrate_legacy_managed_model_root_copies_entries_into_shared_storage() {
        let workspace_dir = tempdir().expect("workspace dir");
        let app_storage_dir = tempdir().expect("app storage dir");
        let legacy_model_dir = managed_model_root(workspace_dir.path()).join("manual-model");
        create_dummy_manual_model_files(&legacy_model_dir);

        migrate_legacy_managed_model_root(workspace_dir.path(), app_storage_dir.path())
            .expect("migrate legacy model root");

        let shared_model_dir = managed_model_root(app_storage_dir.path()).join("manual-model");
        assert!(shared_model_dir.join("model.onnx").is_file());
        assert!(shared_model_dir.join("tokenizer.json").is_file());
    }

    #[test]
    fn prioritize_directml_adapter_indices_prefers_high_performance_then_memory() {
        let ordered = prioritize_directml_adapter_indices(&[
            DirectMlAdapterCandidate {
                index: 0,
                dedicated_video_memory: 8,
                is_software: false,
                is_high_performance: false,
            },
            DirectMlAdapterCandidate {
                index: 1,
                dedicated_video_memory: 1,
                is_software: false,
                is_high_performance: false,
            },
            DirectMlAdapterCandidate {
                index: 2,
                dedicated_video_memory: 4,
                is_software: false,
                is_high_performance: true,
            },
            DirectMlAdapterCandidate {
                index: 3,
                dedicated_video_memory: 0,
                is_software: false,
                is_high_performance: false,
            },
        ]);

        assert_eq!(ordered, vec![2]);
    }

    #[test]
    fn prioritize_directml_adapter_indices_filters_software_adapters() {
        let ordered = prioritize_directml_adapter_indices(&[
            DirectMlAdapterCandidate {
                index: 0,
                dedicated_video_memory: 0,
                is_software: true,
                is_high_performance: true,
            },
            DirectMlAdapterCandidate {
                index: 1,
                dedicated_video_memory: 0,
                is_software: true,
                is_high_performance: false,
            },
        ]);

        assert_eq!(ordered, vec![0]);
    }

    #[test]
    fn prioritize_directml_adapter_indices_falls_back_to_remaining_hardware_when_no_preferred_gpu()
    {
        let ordered = prioritize_directml_adapter_indices(&[
            DirectMlAdapterCandidate {
                index: 0,
                dedicated_video_memory: 6,
                is_software: false,
                is_high_performance: false,
            },
            DirectMlAdapterCandidate {
                index: 1,
                dedicated_video_memory: 0,
                is_software: false,
                is_high_performance: false,
            },
            DirectMlAdapterCandidate {
                index: 2,
                dedicated_video_memory: 2,
                is_software: false,
                is_high_performance: false,
            },
        ]);

        assert_eq!(ordered, vec![0, 2, 1]);
    }
}
