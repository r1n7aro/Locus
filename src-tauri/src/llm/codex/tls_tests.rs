use super::{config_with_roots, server_name};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::{RootCertStore, ServerConfig};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{TlsAcceptor, TlsConnector};

const CERT: &[u8] = include_bytes!("test_fixtures/localhost-cert.der");
const KEY: &[u8] = include_bytes!("test_fixtures/localhost-key.der");

async fn exchange(host: &str, trust_server: bool) -> Result<[u8; 4], std::io::Error> {
    let mut roots = RootCertStore::empty();
    if trust_server {
        roots.add(CertificateDer::from(CERT.to_vec())).unwrap();
    }
    let connector = TlsConnector::from(Arc::new(config_with_roots(roots).unwrap()));
    let server_config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(CERT.to_vec())],
        PrivatePkcs8KeyDer::from(KEY.to_vec()).into(),
    )
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    // In-memory TLS only: no socket, proxy, credentials or external request.
    let (client_io, server_io) = tokio::io::duplex(4096);
    let client = async {
        let mut stream = connector
            .connect(server_name(host).unwrap(), client_io)
            .await?;
        stream.write_all(b"ping").await?;
        let mut reply = [0; 4];
        stream.read_exact(&mut reply).await?;
        Ok(reply)
    };
    let server = async {
        let mut stream = acceptor.accept(server_io).await?;
        let mut request = [0; 4];
        stream.read_exact(&mut request).await?;
        assert_eq!(&request, b"ping");
        stream.write_all(b"pong").await
    };
    let (result, _) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(client, server)
    })
    .await
    .expect("in-memory TLS handshake must finish");
    result
}

#[tokio::test]
async fn trusted_server_can_exchange_encrypted_data() {
    assert_eq!(exchange("localhost", true).await.unwrap(), *b"pong");
}

#[tokio::test]
async fn untrusted_certificate_is_rejected() {
    let error = exchange("localhost", false).await.unwrap_err();
    assert!(
        matches!(
            error
                .get_ref()
                .and_then(|source| source.downcast_ref::<rustls::Error>()),
            Some(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer
            ))
        ),
        "{error}"
    );
}

#[tokio::test]
async fn trusted_certificate_for_wrong_hostname_is_rejected() {
    let error = exchange("different.example", true).await.unwrap_err();
    assert!(
        matches!(
            error
                .get_ref()
                .and_then(|source| source.downcast_ref::<rustls::Error>()),
            Some(rustls::Error::InvalidCertificate(
                rustls::CertificateError::NotValidForNameContext { .. }
            ))
        ),
        "{error}"
    );
}
