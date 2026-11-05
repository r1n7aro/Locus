//! Sealing of device tokens.
//!
//! Android: AES-256-GCM with a non-exportable key in the AndroidKeyStore
//! (`SecureStorePlugin.kt`); only ciphertext reaches the app's files, and app
//! backups are disabled in the manifest. Other targets exist only for
//! development builds and keep a clearly marked base64 form.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[cfg(target_os = "android")]
use tauri::plugin::PluginHandle;

const PLAIN_PREFIX: &str = "plain:";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct CryptArgs<'a> {
    alias: &'a str,
    data: &'a str,
}

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct CryptResult {
    data: String,
}

#[derive(Clone)]
pub enum Sealer {
    #[cfg(target_os = "android")]
    Keystore(PluginHandle<tauri::Wry>),
    /// Development builds on desktop operating systems only.
    Plain,
}

impl Sealer {
    pub async fn seal(&self, alias: &str, secret: &str) -> Result<String, String> {
        match self {
            #[cfg(target_os = "android")]
            Sealer::Keystore(handle) => {
                let handle = handle.clone();
                let (alias, data) = (alias.to_string(), STANDARD.encode(secret.as_bytes()));
                tokio::task::spawn_blocking(move || {
                    handle
                        .run_mobile_plugin::<CryptResult>("seal", CryptArgs { alias: &alias, data: &data })
                        .map(|result| result.data)
                        .map_err(|error| format!("无法加密保存令牌：{error}"))
                })
                .await
                .map_err(|error| error.to_string())?
            }
            Sealer::Plain => {
                let _ = alias;
                eprintln!("[LocusRemote] development build: token stored without hardware protection");
                Ok(format!("{PLAIN_PREFIX}{}", STANDARD.encode(secret.as_bytes())))
            }
        }
    }

    pub async fn open(&self, alias: &str, sealed: &str) -> Result<Zeroizing<String>, String> {
        #[cfg(not(target_os = "android"))]
        let _ = alias;
        if let Some(encoded) = sealed.strip_prefix(PLAIN_PREFIX) {
            if !matches!(self, Sealer::Plain) {
                return Err("令牌存储格式无效，请重新配对".into());
            }
            return decode_utf8(encoded);
        }
        match self {
            #[cfg(target_os = "android")]
            Sealer::Keystore(handle) => {
                let handle = handle.clone();
                let (alias, data) = (alias.to_string(), sealed.to_string());
                let encoded = tokio::task::spawn_blocking(move || {
                    handle
                        .run_mobile_plugin::<CryptResult>("open", CryptArgs { alias: &alias, data: &data })
                        .map(|result| Zeroizing::new(result.data))
                        .map_err(|error| format!("无法读取令牌，请重新配对：{error}"))
                })
                .await
                .map_err(|error| error.to_string())??;
                decode_utf8(&encoded)
            }
            Sealer::Plain => Err("令牌存储格式无效，请重新配对".into()),
        }
    }

    pub async fn forget(&self, alias: &str) {
        match self {
            #[cfg(target_os = "android")]
            Sealer::Keystore(handle) => {
                let handle = handle.clone();
                let alias = alias.to_string();
                let _ = tokio::task::spawn_blocking(move || {
                    handle.run_mobile_plugin::<serde_json::Value>(
                        "forget",
                        CryptArgs { alias: &alias, data: "" },
                    )
                })
                .await;
            }
            Sealer::Plain => {
                let _ = alias;
            }
        }
    }
}

fn decode_utf8(encoded: &str) -> Result<Zeroizing<String>, String> {
    let bytes = Zeroizing::new(STANDARD.decode(encoded).map_err(|_| "令牌格式无效".to_string())?);
    String::from_utf8(bytes.to_vec())
        .map(Zeroizing::new)
        .map_err(|_| "令牌格式无效".to_string())
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn plain_sealer_round_trips_and_marks_its_format() {
        let sealer = Sealer::Plain;
        let sealed = sealer.seal("a", "lrt_secret").await.unwrap();
        assert!(sealed.starts_with(PLAIN_PREFIX));
        assert!(!sealed.contains("lrt_secret"));
        assert_eq!(sealer.open("a", &sealed).await.unwrap().as_str(), "lrt_secret");
        assert!(sealer.open("a", "garbage").await.is_err());
    }
}
