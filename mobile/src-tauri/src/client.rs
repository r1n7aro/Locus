//! HTTP requests to one desktop address (hello, pair, unpair, page assets).

use std::sync::Arc;
use std::time::Duration;

use locus_remote_protocol::{self as protocol, ErrorBody, Hello, PairRequest, PairResponse};
use reqwest::header::{HeaderMap, AUTHORIZATION};
use reqwest::StatusCode;
use tokio::sync::Semaphore;

use crate::tls;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Covers the desktop's 60 s approval window.
const PAIR_TIMEOUT: Duration = Duration::from_secs(90);
/// Page assets fetched at once. The WebView requests a whole module graph in
/// parallel and every HTTP/1.1 request in flight needs its own connection;
/// browsers cap that at 6 per host, and the desktop drops a peer's
/// connections beyond 16.
const MAX_ASSET_REQUESTS: usize = 6;

#[derive(Debug)]
pub enum ClientError {
    /// The certificate does not match the pin (reset desktop or impersonation).
    PinMismatch,
    Unreachable(String),
    /// The desktop answered with an error body.
    Rejected { status: u16, code: String, message: String },
    Protocol(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PinMismatch => f.write_str("证书与配对时不一致：电脑可能已重置证书，或连接被冒充"),
            Self::Unreachable(error) => write!(f, "无法连接：{error}"),
            Self::Rejected { message, .. } => f.write_str(message),
            Self::Protocol(error) => write!(f, "响应无效：{error}"),
        }
    }
}

fn classify(error: reqwest::Error) -> ClientError {
    if tls::is_pin_mismatch(&error) {
        ClientError::PinMismatch
    } else {
        ClientError::Unreachable(error_chain(&error))
    }
}

/// `outer: inner: innermost`, so the root cause (refused, timed out, TLS)
/// is visible instead of reqwest's generic "error sending request".
fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![error.to_string()];
    let mut current = error.source();
    while let Some(source) = current {
        let text = source.to_string();
        if parts.last() != Some(&text) {
            parts.push(text);
        }
        current = source.source();
    }
    parts.join(": ")
}

/// A fully read page asset.
pub struct Asset {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

/// Cheap to clone; clones share the connection pool and the asset slots.
#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    origin: String,
    asset_slots: Arc<Semaphore>,
}

impl HttpClient {
    /// `pinned`: certificate fingerprint for direct addresses, `None` for
    /// CA-validated hostnames.
    pub fn new(origin: &str, pinned: Option<&str>) -> Result<Self, String> {
        let tls = tls::client_config(pinned)?;
        let client = reqwest::Client::builder()
            .use_preconfigured_tls((*tls).clone())
            .http1_only()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(MAX_ASSET_REQUESTS)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            origin: origin.trim_end_matches('/').to_string(),
            asset_slots: Arc::new(Semaphore::new(MAX_ASSET_REQUESTS)),
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    async fn rejected(response: reqwest::Response) -> ClientError {
        let status = response.status().as_u16();
        match response.json::<ErrorBody>().await {
            Ok(body) => ClientError::Rejected { status, code: body.code, message: body.message },
            Err(_) => ClientError::Rejected {
                status,
                code: "http".into(),
                message: format!("HTTP {status}"),
            },
        }
    }

    pub async fn hello(&self) -> Result<Hello, ClientError> {
        let response = self
            .client
            .get(format!("{}{}", self.origin, protocol::HELLO_PATH))
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(classify)?;
        if !response.status().is_success() {
            return Err(Self::rejected(response).await);
        }
        response.json().await.map_err(|error| ClientError::Protocol(error.to_string()))
    }

    pub async fn pair(&self, request: &PairRequest) -> Result<PairResponse, ClientError> {
        let response = self
            .client
            .post(format!("{}{}", self.origin, protocol::PAIR_PATH))
            .json(request)
            .timeout(PAIR_TIMEOUT)
            .send()
            .await
            .map_err(classify)?;
        if !response.status().is_success() {
            return Err(Self::rejected(response).await);
        }
        let paired: PairResponse = response
            .json()
            .await
            .map_err(|error| ClientError::Protocol(error.to_string()))?;
        if !protocol::is_token(&paired.token) {
            return Err(ClientError::Protocol("malformed token".into()));
        }
        Ok(paired)
    }

    pub async fn unpair(&self, token: &str) -> Result<(), ClientError> {
        let response = self
            .client
            .post(format!("{}{}", self.origin, protocol::UNPAIR_PATH))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(classify)?;
        if response.status().is_success() || response.status() == StatusCode::UNAUTHORIZED {
            Ok(())
        } else {
            Err(Self::rejected(response).await)
        }
    }

    /// Authenticated GET of a page asset, read in full for the caller to relay.
    pub async fn get_asset(&self, path_and_query: &str, token: &str) -> Result<Asset, ClientError> {
        let _slot = self
            .asset_slots
            .acquire()
            .await
            .map_err(|_| ClientError::Unreachable("closed".into()))?;
        match self.get_asset_once(path_and_query, token).await {
            Ok(asset) => Ok(asset),
            // GET is idempotent: one retry covers a pooled connection the
            // desktop closed meanwhile. A timeout or a pin mismatch would
            // only repeat.
            Err(error) if !error.is_timeout() && !tls::is_pin_mismatch(&error) => {
                self.get_asset_once(path_and_query, token).await.map_err(classify)
            }
            Err(error) => Err(classify(error)),
        }
    }

    async fn get_asset_once(&self, path_and_query: &str, token: &str) -> Result<Asset, reqwest::Error> {
        let response = self
            .client
            .get(format!("{}{}", self.origin, path_and_query))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = Vec::from(response.bytes().await?);
        Ok(Asset { status, headers, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serves a fixed certificate and signing key, even when they do not
    /// belong together.
    #[derive(Debug)]
    struct FixedCert(Arc<rustls::sign::CertifiedKey>);

    impl rustls::server::ResolvesServerCert for FixedCert {
        fn resolve(&self, _: rustls::server::ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
            Some(self.0.clone())
        }
    }

    /// TLS 1.3 server with a fresh self-signed certificate answering every
    /// request with a hello body. Returns its origin and certificate pin.
    /// `impostor`: presents the certificate but signs the handshake with an
    /// unrelated key, like an attacker who copied the public certificate.
    async fn tls_server(impostor: bool) -> (String, String) {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap().self_signed(&key).unwrap();
        let fingerprint = protocol::certificate_fingerprint(cert.der());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let signing_der = if impostor {
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap().serialize_der()
        } else {
            key.serialize_der()
        };
        let signing_key = provider
            .key_provider
            .load_private_key(rustls::pki_types::PrivateKeyDer::Pkcs8(signing_der.into()))
            .unwrap();
        let certified = Arc::new(rustls::sign::CertifiedKey::new(vec![cert.der().clone()], signing_key));
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(FixedCert(certified)));
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(stream).await else { return };
                    let mut request = [0u8; 4096];
                    let _ = tls.read(&mut request).await;
                    let body = r#"{"protocol":1,"desktopId":"d","pairing":false}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = tls.write_all(response.as_bytes()).await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        (format!("https://{address}"), fingerprint)
    }

    #[tokio::test]
    async fn pinned_certificate_is_trusted_and_anything_else_is_refused() {
        let (origin, fingerprint) = tls_server(false).await;
        let hello = HttpClient::new(&origin, Some(&fingerprint)).unwrap().hello().await.unwrap();
        assert_eq!(hello.desktop_id, "d");

        let error = HttpClient::new(&origin, Some(&"0".repeat(64))).unwrap().hello().await.unwrap_err();
        assert!(matches!(error, ClientError::PinMismatch), "{error:?}");

        // WebPKI validation refuses a self-signed certificate outright.
        let error = HttpClient::new(&origin, None).unwrap().hello().await.unwrap_err();
        assert!(matches!(error, ClientError::Unreachable(_)), "{error:?}");
    }

    #[tokio::test]
    async fn replayed_certificate_without_its_key_is_refused() {
        // The pin matches; the handshake signature cannot.
        let (origin, fingerprint) = tls_server(true).await;
        let error = HttpClient::new(&origin, Some(&fingerprint)).unwrap().hello().await.unwrap_err();
        assert!(matches!(error, ClientError::PinMismatch), "{error:?}");
    }
}
