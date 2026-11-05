//! Per-install self-signed certificate for the NIC listeners.
//!
//! Phones never validate it against a CA: the Locus Remote app pins the
//! SHA-256 of this exact certificate, delivered inside the pairing QR code.
//! The certificate therefore has a long validity and no SANs, and changes only
//! when the user resets it (which invalidates every pairing).

use std::path::PathBuf;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

const CERT_FILE: &str = "remote_access_cert.der";
const KEY_FALLBACK_FILE: &str = "remote_access_key.der";
const KEY_SECRET: &str = "locus-remote-access/tls-key";

pub struct Identity {
    pub cert_der: Vec<u8>,
    key_der: Vec<u8>,
    pub fingerprint: String,
}

impl Identity {
    fn new(cert_der: Vec<u8>, key_der: Vec<u8>) -> Self {
        let fingerprint = locus_remote_protocol::certificate_fingerprint(&cert_der);
        Self { cert_der, key_der, fingerprint }
    }

    pub fn server_config(&self) -> Result<Arc<rustls::ServerConfig>, String> {
        // Every client is the Locus Remote app (rustls), so TLS 1.3 only.
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|error| format!("TLS 配置失败：{error}"))?
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(self.cert_der.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone())),
            )
            .map_err(|error| format!("TLS 证书无效：{error}"))?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }
}

fn config_dir() -> Result<PathBuf, String> {
    crate::commands::persistent_config_dir()
}

/// Loads the persisted identity, creating one on first use.
pub fn load_or_create() -> Result<Identity, String> {
    let dir = config_dir()?;
    if let Ok(cert_der) = std::fs::read(dir.join(CERT_FILE)) {
        if let Some(key_der) = load_key(&dir) {
            if key_matches_certificate(&cert_der, &key_der) {
                return Ok(Identity::new(cert_der, key_der));
            }
            eprintln!("[RemoteAccess] stored TLS key does not match the certificate; regenerating");
        }
    }
    regenerate()
}

/// Generates and persists a fresh identity. Every pinned client must pair
/// again afterwards.
pub fn regenerate() -> Result<Identity, String> {
    let (cert_der, key_der) = generate()?;
    let dir = config_dir()?;
    store_key(&dir, &key_der)?;
    crate::config::atomic_write_config(&dir.join(CERT_FILE), &cert_der)?;
    Ok(Identity::new(cert_der, key_der))
}

fn generate() -> Result<(Vec<u8>, Vec<u8>), String> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|error| format!("无法生成 TLS 密钥：{error}"))?;
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new())
        .map_err(|error| format!("无法生成 TLS 证书：{error}"))?;
    let mut name = rcgen::DistinguishedName::new();
    name.push(rcgen::DnType::CommonName, "Locus Remote Access");
    params.distinguished_name = name;
    params.not_before = rcgen::date_time_ymd(2025, 1, 1);
    params.not_after = rcgen::date_time_ymd(2125, 1, 1);
    let cert = params
        .self_signed(&key)
        .map_err(|error| format!("无法签发 TLS 证书：{error}"))?;
    Ok((cert.der().to_vec(), key.serialize_der()))
}

fn load_key(dir: &std::path::Path) -> Option<Vec<u8>> {
    match crate::keychain::get_secret(KEY_SECRET) {
        Ok(Some(encoded)) => return STANDARD.decode(encoded.trim()).ok(),
        Ok(None) => {}
        Err(error) => eprintln!("[RemoteAccess] keychain read failed: {error}"),
    }
    std::fs::read(dir.join(KEY_FALLBACK_FILE)).ok()
}

fn store_key(dir: &std::path::Path, key_der: &[u8]) -> Result<(), String> {
    let fallback = dir.join(KEY_FALLBACK_FILE);
    match crate::keychain::set_secret(KEY_SECRET, &STANDARD.encode(key_der)) {
        Ok(()) => {
            let _ = std::fs::remove_file(fallback);
            Ok(())
        }
        Err(error) => {
            eprintln!(
                "[RemoteAccess] keychain unavailable ({error}); storing the TLS key in the config directory"
            );
            crate::config::atomic_write_config(&fallback, key_der)
        }
    }
}

/// Guards against a certificate file and key from different generations.
fn key_matches_certificate(cert_der: &[u8], key_der: &[u8]) -> bool {
    use rcgen::PublicKeyData;
    let Ok(key) = rcgen::KeyPair::try_from(key_der) else {
        return false;
    };
    let public = key.subject_public_key_info();
    !public.is_empty() && contains(cert_der, &public)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_identity_builds_a_server_config_and_matches_its_key() {
        let (cert_der, key_der) = generate().unwrap();
        assert!(key_matches_certificate(&cert_der, &key_der));
        let (other_cert, _) = generate().unwrap();
        assert!(!key_matches_certificate(&other_cert, &key_der));
        let identity = Identity::new(cert_der, key_der);
        assert!(locus_remote_protocol::is_fingerprint(&identity.fingerprint));
        identity.server_config().unwrap();
    }
}
