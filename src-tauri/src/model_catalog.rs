//! models.dev model catalog: embedded snapshot + on-disk cache + background refresh.
//!
//! Data layering (freshest wins by `fetched_at`):
//! 1. On-disk cache `{persistent_config_dir}/model_catalog.json`, written by refresh.
//! 2. Embedded snapshot `assets/model_catalog.json.gz`, checked in and updated via
//!    `bun run catalog:refresh` (scripts/refresh-model-catalog.mjs — keep the slim
//!    schema in sync with that script and src/types.ts).
//!
//! Refresh fetches the full https://models.dev/api.json, slims it to the fields
//! Locus consumes, and swaps the cache atomically. Failures are non-fatal: the
//! catalog silently falls back to the freshest local data.

use std::io::Read;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const EMBEDDED_SNAPSHOT_GZ: &[u8] = include_bytes!("../assets/model_catalog.json.gz");
const CACHE_FILE_NAME: &str = "model_catalog.json";
const DEFAULT_SOURCE_URL: &str = "https://models.dev/api.json";
const SOURCE_URL_ENV: &str = "LOCUS_MODELS_URL";
const REFRESH_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Refuse to persist a refresh result that lost most of the catalog — a broken
/// mirror or a truncated response must not clobber a good snapshot.
const MIN_SANE_PROVIDERS: usize = 50;
const MIN_SANE_MODELS: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogLimit {
    #[serde(default)]
    pub context: u64,
    #[serde(default)]
    pub output: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogModel {
    pub name: String,
    pub limit: CatalogLimit,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tool_call: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub attachment: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub temperature: bool,
    /// models.dev `interleaved`: `true` or `{ "field": "reasoning_content" | ... }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interleaved: Option<Value>,
    /// models.dev `reasoning_options` passed through verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_options: Option<Value>,
    /// Slimmed to `{ "input": [...] }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogProvider {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default)]
    pub models: indexmap::IndexMap<String, CatalogModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub version: u32,
    pub fetched_at: String,
    pub providers: indexmap::IndexMap<String, CatalogProvider>,
}

impl ModelCatalog {
    fn model_count(&self) -> usize {
        self.providers.values().map(|p| p.models.len()).sum()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogResponse {
    /// "cache" (refreshed at runtime) or "snapshot" (embedded build-time data).
    pub source: String,
    pub fetched_at: String,
    pub providers: indexmap::IndexMap<String, CatalogProvider>,
}

fn source_url() -> String {
    match std::env::var(SOURCE_URL_ENV) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => DEFAULT_SOURCE_URL.to_string(),
    }
}

fn cache_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join(CACHE_FILE_NAME))
}

fn parse_embedded_snapshot() -> Result<ModelCatalog, String> {
    let mut decoder = flate2::read::GzDecoder::new(EMBEDDED_SNAPSHOT_GZ);
    let mut json = String::new();
    decoder
        .read_to_string(&mut json)
        .map_err(|e| format!("Failed to decompress embedded model catalog: {e}"))?;
    serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse embedded model catalog: {e}"))
}

fn load_cached_catalog() -> Option<ModelCatalog> {
    let path = cache_path().ok()?;
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

struct CatalogState {
    catalog: Arc<ModelCatalog>,
    source: &'static str,
}

fn catalog_cell() -> &'static tokio::sync::RwLock<Option<Arc<CatalogState>>> {
    static CELL: OnceLock<tokio::sync::RwLock<Option<Arc<CatalogState>>>> = OnceLock::new();
    CELL.get_or_init(|| tokio::sync::RwLock::new(None))
}

fn refresh_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Freshest of disk cache vs embedded snapshot; ISO-8601 strings compare lexically.
fn load_freshest() -> Result<CatalogState, String> {
    let snapshot = parse_embedded_snapshot()?;
    match load_cached_catalog() {
        Some(cached) if cached.fetched_at.as_str() > snapshot.fetched_at.as_str() => {
            Ok(CatalogState {
                catalog: Arc::new(cached),
                source: "cache",
            })
        }
        _ => Ok(CatalogState {
            catalog: Arc::new(snapshot),
            source: "snapshot",
        }),
    }
}

async fn current_state() -> Result<Arc<CatalogState>, String> {
    if let Some(state) = catalog_cell().read().await.as_ref() {
        return Ok(state.clone());
    }
    let mut guard = catalog_cell().write().await;
    if let Some(state) = guard.as_ref() {
        return Ok(state.clone());
    }
    let state = Arc::new(load_freshest()?);
    *guard = Some(state.clone());
    Ok(state)
}

fn slim_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn slim_model(id: &str, raw: &Value) -> CatalogModel {
    let limit = raw.get("limit");
    let cost = raw.get("cost").and_then(Value::as_object);
    CatalogModel {
        name: slim_string(raw, "name").unwrap_or_else(|| id.to_string()),
        limit: CatalogLimit {
            context: limit
                .and_then(|l| l.get("context"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output: limit
                .and_then(|l| l.get("output"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
        reasoning: raw.get("reasoning").and_then(Value::as_bool).unwrap_or(false),
        tool_call: raw.get("tool_call").and_then(Value::as_bool).unwrap_or(false),
        attachment: raw.get("attachment").and_then(Value::as_bool).unwrap_or(false),
        temperature: raw
            .get("temperature")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        interleaved: raw.get("interleaved").cloned(),
        reasoning_options: raw
            .get("reasoning_options")
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
            .cloned(),
        modalities: raw
            .get("modalities")
            .and_then(|m| m.get("input"))
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
            .map(|input| serde_json::json!({ "input": input })),
        release_date: slim_string(raw, "release_date"),
        status: slim_string(raw, "status"),
        cost: cost
            .filter(|c| {
                c.get("input").and_then(Value::as_f64).unwrap_or(0.0) != 0.0
                    || c.get("output").and_then(Value::as_f64).unwrap_or(0.0) != 0.0
            })
            .map(|c| {
                let mut slim = serde_json::Map::new();
                for key in ["input", "output", "cache_read", "cache_write"] {
                    if let Some(v) = c.get(key) {
                        slim.insert(key.to_string(), v.clone());
                    }
                }
                Value::Object(slim)
            }),
    }
}

fn slim_catalog(raw: &Value) -> Result<ModelCatalog, String> {
    let map = raw
        .as_object()
        .ok_or_else(|| "models.dev api.json is not an object".to_string())?;
    let mut providers = indexmap::IndexMap::new();
    for (pid, provider) in map {
        let Some(models) = provider.get("models").and_then(Value::as_object) else {
            continue;
        };
        let mut slim_models = indexmap::IndexMap::new();
        for (mid, model) in models {
            slim_models.insert(mid.clone(), slim_model(mid, model));
        }
        providers.insert(
            pid.clone(),
            CatalogProvider {
                name: slim_string(provider, "name").unwrap_or_else(|| pid.clone()),
                api: slim_string(provider, "api"),
                npm: slim_string(provider, "npm"),
                env: provider.get("env").and_then(|v| {
                    let list: Vec<String> = v
                        .as_array()?
                        .iter()
                        .filter_map(|e| e.as_str().map(str::to_string))
                        .collect();
                    (!list.is_empty()).then_some(list)
                }),
                doc: slim_string(provider, "doc"),
                models: slim_models,
            },
        );
    }
    Ok(ModelCatalog {
        version: 1,
        fetched_at: chrono::Utc::now().to_rfc3339(),
        providers,
    })
}

fn write_cache(catalog: &ModelCatalog) -> Result<(), String> {
    let path = cache_path()?;
    let json = serde_json::to_string(catalog)
        .map_err(|e| format!("Failed to serialize model catalog: {e}"))?;
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    std::fs::write(&tmp, json).map_err(|e| format!("Failed to write model catalog cache: {e}"))?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Failed to swap model catalog cache: {e}"));
    }
    Ok(())
}

fn cache_is_fresh() -> bool {
    let Some(cached) = load_cached_catalog() else {
        return false;
    };
    let Ok(fetched) = chrono::DateTime::parse_from_rfc3339(&cached.fetched_at) else {
        return false;
    };
    let age = chrono::Utc::now().signed_duration_since(fetched.with_timezone(&chrono::Utc));
    age.to_std().map(|d| d < REFRESH_TTL).unwrap_or(true)
}

async fn fetch_and_store() -> Result<ModelCatalog, String> {
    let url = source_url();
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .timeout(FETCH_TIMEOUT)
            .user_agent(format!("locus/{}", env!("CARGO_PKG_VERSION")))
            .gzip(true),
    )?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch model catalog from {url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Model catalog fetch failed: HTTP {} from {url}",
            response.status()
        ));
    }
    let raw: Value = response
        .json()
        .await
        .map_err(|e| format!("Model catalog response is not valid JSON: {e}"))?;
    let catalog = slim_catalog(&raw)?;
    let providers = catalog.providers.len();
    let models = catalog.model_count();
    if providers < MIN_SANE_PROVIDERS || models < MIN_SANE_MODELS {
        return Err(format!(
            "Model catalog sanity check failed: {providers} providers / {models} models from {url}"
        ));
    }
    write_cache(&catalog)?;
    Ok(catalog)
}

/// Refresh the catalog from the network. `force=false` respects the 24h TTL.
pub async fn refresh(force: bool) -> Result<(), String> {
    let _guard = refresh_lock().lock().await;
    if !force && cache_is_fresh() {
        return Ok(());
    }
    let catalog = fetch_and_store().await?;
    let state = Arc::new(CatalogState {
        catalog: Arc::new(catalog),
        source: "cache",
    });
    *catalog_cell().write().await = Some(state);
    Ok(())
}

/// Spawned once at startup; failures only log — local data keeps serving.
pub async fn background_refresh() {
    if let Err(error) = refresh(false).await {
        eprintln!("[Locus] model catalog background refresh skipped: {error}");
    }
}

async fn catalog_response() -> Result<ModelCatalogResponse, String> {
    let state = current_state().await?;
    Ok(ModelCatalogResponse {
        source: state.source.to_string(),
        fetched_at: state.catalog.fetched_at.clone(),
        providers: state.catalog.providers.clone(),
    })
}

#[tauri::command]
pub async fn get_model_catalog() -> Result<ModelCatalogResponse, String> {
    catalog_response().await
}

#[tauri::command]
pub async fn refresh_model_catalog() -> Result<ModelCatalogResponse, String> {
    refresh(true).await?;
    catalog_response().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_snapshot_parses_and_is_populated() {
        let snapshot = parse_embedded_snapshot().expect("embedded snapshot must parse");
        assert_eq!(snapshot.version, 1);
        assert!(snapshot.providers.len() >= MIN_SANE_PROVIDERS);
        assert!(snapshot.model_count() >= MIN_SANE_MODELS);
        assert!(!snapshot.fetched_at.is_empty());
    }

    #[test]
    fn embedded_snapshot_has_expected_provider_shape() {
        let snapshot = parse_embedded_snapshot().expect("embedded snapshot must parse");
        let deepseek = snapshot
            .providers
            .get("deepseek")
            .expect("deepseek provider present");
        assert_eq!(deepseek.npm.as_deref(), Some("@ai-sdk/openai-compatible"));
        assert!(deepseek.api.as_deref().is_some_and(|a| a.contains("deepseek")));
        let (_, model) = deepseek
            .models
            .iter()
            .next()
            .expect("deepseek has at least one model");
        assert!(model.limit.context > 0);
    }

    #[test]
    fn slim_catalog_extracts_expected_fields() {
        let raw = serde_json::json!({
            "prov": {
                "name": "Prov",
                "api": "https://api.example.com/v1",
                "npm": "@ai-sdk/openai-compatible",
                "env": ["PROV_API_KEY"],
                "models": {
                    "m1": {
                        "name": "Model One",
                        "reasoning": true,
                        "tool_call": true,
                        "interleaved": { "field": "reasoning_content" },
                        "reasoning_options": [ { "type": "effort", "values": ["low", "high"] } ],
                        "modalities": { "input": ["text", "image"], "output": ["text"] },
                        "limit": { "context": 128000, "output": 8192 },
                        "cost": { "input": 1.0, "output": 2.0 },
                        "extra_unknown_field": { "ignored": true }
                    }
                }
            }
        });
        let catalog = slim_catalog(&raw).expect("slims");
        let model = &catalog.providers["prov"].models["m1"];
        assert!(model.reasoning && model.tool_call);
        assert_eq!(model.limit.context, 128000);
        assert_eq!(
            model
                .interleaved
                .as_ref()
                .and_then(|v| v.get("field"))
                .and_then(Value::as_str),
            Some("reasoning_content")
        );
        assert!(model.reasoning_options.is_some());
        assert_eq!(
            model
                .modalities
                .as_ref()
                .and_then(|m| m.get("input"))
                .and_then(Value::as_array)
                .map(|a| a.len()),
            Some(2)
        );
    }
}
