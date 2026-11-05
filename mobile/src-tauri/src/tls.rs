//! TLS client configurations.
//!
//! Direct (LAN/Tailscale) addresses present the desktop's self-signed
//! certificate. It is trusted by exact SHA-256 match with the fingerprint
//! from the pairing QR code; hostname and validity dates are ignored because
//! the pin already identifies the certificate. The handshake signature is
//! still verified, so presenting the (public) certificate without its private
//! key fails. Other addresses use normal WebPKI validation.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[derive(Debug)]
struct PinnedCertificate {
    fingerprint: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if locus_remote_protocol::certificate_fingerprint(end_entity.as_ref()) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// `Some(fingerprint)`: trust exactly that certificate. `None`: WebPKI roots.
pub fn client_config(pinned_fingerprint: Option<&str>) -> Result<Arc<ClientConfig>, String> {
    let provider = provider();
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| format!("TLS 配置失败：{error}"))?;
    let mut config = match pinned_fingerprint {
        Some(fingerprint) => {
            if !locus_remote_protocol::is_fingerprint(fingerprint) {
                return Err("证书指纹无效".into());
            }
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinnedCertificate {
                    fingerprint: fingerprint.to_string(),
                    provider,
                }))
                .with_no_client_auth()
        }
        None => {
            let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// True when a connection failed because the peer is not the paired
/// desktop: the certificate does not match the pin (reset certificate or
/// impersonation), or it matches but the handshake was not signed with its
/// key (a replayed certificate). Never retried silently with weaker checks.
pub fn is_pin_mismatch(error: &(dyn std::error::Error + 'static)) -> bool {
    matches!(
        find_rustls_error(error),
        Some(Error::InvalidCertificate(
            CertificateError::ApplicationVerificationFailure | CertificateError::BadSignature
        ))
    )
}

/// Finds the typed rustls error however the HTTP/WebSocket stack wrapped it.
/// `io::Error` and `Box<dyn Error>` are unwrapped explicitly: the standard
/// `source()` of a boxed error skips the boxed value itself, so following
/// `source()` alone never reaches the rustls error inside it.
fn find_rustls_error<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a Error> {
    if let Some(found) = error.downcast_ref::<Error>() {
        return Some(found);
    }
    if let Some(io) = error.downcast_ref::<std::io::Error>() {
        if let Some(found) = io.get_ref().and_then(|inner| find_rustls_error(inner)) {
            return Some(found);
        }
    }
    error.source().and_then(find_rustls_error)
}
