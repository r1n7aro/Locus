//! Per-project compile-parameter cache, synced from the Unity Editor.
//!
//! Before every sidecar compile, `get_params` does one cheap pipe roundtrip:
//! Unity re-hashes its current reference set (path list + per-file
//! mtime/size + defines + language version) and answers `unchanged` when the
//! hash matches `known_fingerprint`, or with the full parameter set
//! otherwise. The `domain_generation` GUID (minted per AppDomain load) is
//! returned in both cases — it keys the sidecar's in-memory image registry.
//!
//! The cache is additionally dropped from the Rust side at the points that
//! already invalidate the Unity type index (recompile completion, domain
//! reload reconnect), purely to keep the next roundtrip honest if Unity's
//! own change tracking ever misses an update.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;
use tokio::sync::Mutex;

use super::CompileParams;

fn params_cache() -> &'static Mutex<HashMap<String, CompileParams>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CompileParams>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn project_key(project_path: &str) -> String {
    project_path
        .strip_prefix(r"\\?\")
        .unwrap_or(project_path)
        .trim()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

/// Wire shape of the Unity `get_compile_params` response (JsonUtility on the
/// Unity side, hence snake_case field names and no missing-field nulls).
#[derive(Debug, Deserialize)]
struct GetCompileParamsResponse {
    #[serde(default)]
    unchanged: bool,
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    domain_generation: String,
    #[serde(default)]
    lang_version: String,
    #[serde(default)]
    reference_paths: Vec<String>,
    #[serde(default)]
    defines: Vec<String>,
}

/// Get current compile params for `project_path`, refreshing from Unity.
/// Errors are transport-level (Unity unreachable / malformed response) and
/// should route the caller to the legacy in-Unity compile path.
pub async fn get_params(project_path: &str) -> Result<CompileParams, String> {
    let key = project_key(project_path);
    let cached = { params_cache().lock().await.get(&key).cloned() };

    let known_fingerprint = cached
        .as_ref()
        .map(|params| params.fingerprint.clone())
        .unwrap_or_default();

    let payload = serde_json::json!({ "known_fingerprint": known_fingerprint }).to_string();
    let resp =
        crate::unity_bridge::send_message(project_path, "get_compile_params", &payload).await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "get_compile_params failed".to_string()));
    }

    let message = resp.message.unwrap_or_default();
    let response: GetCompileParamsResponse = serde_json::from_str(&message)
        .map_err(|e| format!("get_compile_params response parse failed: {e}"))?;

    if response.domain_generation.trim().is_empty() {
        return Err("get_compile_params response missing domain_generation".to_string());
    }

    let params = if response.unchanged {
        match cached {
            // Same reference params; the domain generation may still have
            // moved (reload without a reference change).
            Some(mut params) => {
                params.domain_generation = response.domain_generation;
                params
            }
            None => {
                return Err(
                    "get_compile_params returned unchanged without a cached parameter set"
                        .to_string(),
                )
            }
        }
    } else {
        if response.fingerprint.trim().is_empty() || response.reference_paths.is_empty() {
            return Err("get_compile_params response missing reference data".to_string());
        }
        CompileParams {
            fingerprint: response.fingerprint,
            domain_generation: response.domain_generation,
            lang_version: response.lang_version,
            reference_paths: response.reference_paths,
            defines: response.defines,
        }
    };

    params_cache()
        .lock()
        .await
        .insert(key, params.clone());
    Ok(params)
}

/// Drop the cached params so the next compile re-fetches the full set.
pub async fn invalidate(project_path: &str) {
    let key = project_key(project_path);
    params_cache().lock().await.remove(&key);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_parses_full_payload() {
        let json = r#"{
            "unchanged": false,
            "fingerprint": "abc",
            "domain_generation": "11112222333344445555666677778888",
            "lang_version": "9",
            "reference_paths": ["C:/proj/Library/ScriptAssemblies/Assembly-CSharp.dll"],
            "defines": ["UNITY_EDITOR", "UNITY_2022_3_OR_NEWER"]
        }"#;
        let response: GetCompileParamsResponse = serde_json::from_str(json).expect("parse");
        assert!(!response.unchanged);
        assert_eq!(response.fingerprint, "abc");
        assert_eq!(response.reference_paths.len(), 1);
        assert_eq!(response.defines.len(), 2);
    }

    #[test]
    fn response_parses_unchanged_payload() {
        let json = r#"{ "unchanged": true, "fingerprint": "abc", "domain_generation": "gen" }"#;
        let response: GetCompileParamsResponse = serde_json::from_str(json).expect("parse");
        assert!(response.unchanged);
        assert!(response.reference_paths.is_empty());
    }

    #[test]
    fn project_keys_normalize() {
        assert_eq!(
            project_key(r"\\?\C:\Proj\Game"),
            project_key("c:/proj/game")
        );
    }
}
