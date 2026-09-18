//! Codex WebSocket TLS, including TLS to an HTTPS proxy.
//! Keep the system trust roots while selecting the same crypto provider as Codex.

use std::sync::{Arc, OnceLock};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

pub(super) fn connector() -> Result<TlsConnector, String> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if let Some(config) = CONFIG.get() {
        return Ok(TlsConnector::from(Arc::clone(config)));
    }

    let certificates = rustls_native_certs::load_native_certs();
    if !certificates.errors.is_empty() {
        tracing::warn!(
            error_count = certificates.errors.len(),
            "Some system TLS certificates could not be loaded"
        );
    }
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(certificates.certs);
    if roots.is_empty() {
        return Err("Failed to load system TLS trust roots for Codex".to_string());
    }
    let config = Arc::new(config_with_roots(roots)?);
    Ok(TlsConnector::from(Arc::clone(
        CONFIG.get_or_init(|| config),
    )))
}

fn config_with_roots(roots: RootCertStore) -> Result<ClientConfig, String> {
    // Select explicitly: other dependencies can enable both ring and aws-lc-rs.
    // Do not change a crypto provider installed by another part of the process.
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("Failed to configure Codex TLS: {error}"))?
        .with_root_certificates(roots)
        .with_no_client_auth())
}

pub(super) fn server_name(host: &str) -> Result<ServerName<'static>, String> {
    ServerName::try_from(host.to_owned())
        .map_err(|error| format!("Invalid TLS server name for Codex: {error}"))
}

#[cfg(test)]
#[path = "tls_tests.rs"]
mod tests;
