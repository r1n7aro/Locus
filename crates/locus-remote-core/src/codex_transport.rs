//! The same outbound WebSocket/TCP/TLS implementation used by the desktop.
use http::Uri;
use hyper_util::client::legacy::connect::{proxy::SocksV4, HttpConnector};
use hyper_util::client::proxy::matcher::Intercept;
use std::{io, time::Duration};
use tokio_tungstenite::proxy::connect_via_proxy;
use tokio_tungstenite::tungstenite::extensions::{
    compression::deflate::DeflateConfig, ExtensionsConfig,
};
use tokio_tungstenite::tungstenite::proxy::{
    ProxyAuth as TungsteniteProxyAuth, ProxyConfig as TungsteniteProxyConfig,
    ProxyScheme as TungsteniteProxyScheme,
};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Error as WsError};
use tower_service::Service;
#[path = "codex_tls.rs"]
pub mod tls;
pub const WEBSOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const ORIGINATOR: &str = "opencode";
pub type WebSocket = tokio_tungstenite::WebSocketStream<BoxedCodexIo>;

pub async fn connect(
    request: http::Request<()>,
) -> Result<(WebSocket, http::Response<Option<Vec<u8>>>), WsError> {
    let transport = connect_websocket_transport(
        &request,
        &hyper_util::client::proxy::matcher::Matcher::from_system(),
    )
    .await
    .map_err(ws_io_error)?;
    let transport = wrap_websocket_transport_tls(&request, transport)
        .await
        .map_err(ws_io_error)?;
    tokio_tungstenite::client_async_with_config(request, transport, Some(websocket_config())).await
}
pub trait CodexAsyncIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

impl<T> CodexAsyncIo for T where T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

pub type BoxedCodexIo = Box<dyn CodexAsyncIo>;

fn authority_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{}]", host)
    } else {
        host.to_string()
    }
}

pub fn websocket_proxy_match_uri(uri: &Uri) -> Result<Uri, String> {
    let scheme = match uri.scheme_str() {
        Some("ws") => "http",
        Some("wss") => "https",
        Some(other) => {
            return Err(format!(
                "Unsupported websocket endpoint scheme for proxy matching: {}",
                other
            ));
        }
        None => return Err("Websocket endpoint is missing a scheme".to_string()),
    };

    let authority = uri
        .authority()
        .cloned()
        .ok_or_else(|| "Websocket endpoint is missing an authority".to_string())?;
    let path_and_query = uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| http::uri::PathAndQuery::from_static("/"));

    http::Uri::builder()
        .scheme(scheme)
        .authority(authority)
        .path_and_query(path_and_query)
        .build()
        .map_err(|e| format!("Failed to build proxy match URI: {}", e))
}

pub fn uri_host_port(uri: &Uri) -> Result<(String, u16), String> {
    let host = uri
        .host()
        .ok_or_else(|| "URI is missing host".to_string())?
        .to_string();
    let port = match uri.port_u16() {
        Some(port) => port,
        None => match uri.scheme_str() {
            Some("http") | Some("ws") => 80,
            Some("https") | Some("wss") => 443,
            Some("socks4") | Some("socks4a") | Some("socks5") | Some("socks5h") => 1080,
            Some(other) => {
                return Err(format!(
                    "Unsupported URI scheme for port resolution: {}",
                    other
                ));
            }
            None => return Err("URI is missing a scheme".to_string()),
        },
    };
    Ok((host, port))
}

pub fn uri_with_resolved_port(uri: &Uri) -> Result<Uri, String> {
    let scheme = uri
        .scheme_str()
        .ok_or_else(|| "URI is missing a scheme".to_string())?;
    let (host, port) = uri_host_port(uri)?;
    let authority = match uri.authority() {
        Some(authority) if authority.as_str().contains('@') => {
            let userinfo = authority
                .as_str()
                .split('@')
                .next()
                .ok_or_else(|| "URI authority is invalid".to_string())?;
            format!("{userinfo}@{}:{}", authority_host(&host), port)
        }
        _ => format!("{}:{}", authority_host(&host), port),
    };
    let path_and_query = uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| http::uri::PathAndQuery::from_static("/"));

    http::Uri::builder()
        .scheme(scheme)
        .authority(authority)
        .path_and_query(path_and_query)
        .build()
        .map_err(|e| format!("Failed to normalize URI port: {}", e))
}

fn build_tcp_connector() -> HttpConnector {
    let mut connector = HttpConnector::new();
    connector.enforce_http(false);
    connector.set_connect_timeout(Some(Duration::from_secs(30)));
    connector.set_keepalive(Some(Duration::from_secs(20)));
    connector.set_keepalive_interval(Some(Duration::from_secs(15)));
    connector.set_keepalive_retries(Some(3));
    connector.set_nodelay(true);
    connector
}

pub fn ws_io_error(message: impl Into<String>) -> WsError {
    WsError::Io(io::Error::other(message.into()))
}

fn proxy_display_uri(uri: &Uri) -> String {
    let scheme = uri.scheme_str().unwrap_or("http");
    let host = uri.host().unwrap_or("<missing-host>");
    match uri.port_u16() {
        Some(port) => format!("{}://{}:{}", scheme, authority_host(host), port),
        None => format!("{}://{}", scheme, authority_host(host)),
    }
}

async fn connect_tcp_stream(uri: &Uri) -> Result<tokio::net::TcpStream, String> {
    let normalized_uri = uri_with_resolved_port(uri)?;
    let mut connector = build_tcp_connector();
    let connection = connector
        .call(normalized_uri.clone())
        .await
        .map_err(|e| format!("Failed to connect to {}: {}", normalized_uri, e))?;
    Ok(connection.into_inner())
}

fn tungstenite_proxy_config(
    proxy: &Intercept,
    scheme: TungsteniteProxyScheme,
) -> Result<TungsteniteProxyConfig, String> {
    let (host, port) = uri_host_port(proxy.uri())?;
    let auth = proxy
        .raw_auth()
        .map(|(username, password)| TungsteniteProxyAuth {
            username: username.to_string(),
            password: password.to_string(),
        });
    Ok(TungsteniteProxyConfig {
        scheme,
        host,
        port,
        auth,
    })
}

async fn connect_via_http_proxy(
    target_uri: &Uri,
    proxy: &Intercept,
) -> Result<BoxedCodexIo, String> {
    let proxy_uri = uri_with_resolved_port(proxy.uri())?;
    eprintln!(
        "[OpenAI Codex][websocket] SYSTEM_PROXY {}",
        proxy_display_uri(&proxy_uri)
    );

    let (target_host, target_port) = uri_host_port(target_uri)?;
    let proxy_config = tungstenite_proxy_config(proxy, TungsteniteProxyScheme::Http)?;
    let tcp = connect_tcp_stream(&proxy_uri).await?;
    let tunneled = connect_via_proxy(tcp, &proxy_config, &target_host, target_port)
        .await
        .map_err(|e| format!("Failed to establish HTTP proxy tunnel: {}", e))?;
    Ok(Box::new(tunneled))
}

async fn connect_via_https_proxy(
    target_uri: &Uri,
    proxy: &Intercept,
) -> Result<BoxedCodexIo, String> {
    let proxy_uri = uri_with_resolved_port(proxy.uri())?;
    eprintln!(
        "[OpenAI Codex][websocket] SYSTEM_PROXY {}",
        proxy_display_uri(&proxy_uri)
    );

    let (proxy_host, _) = uri_host_port(&proxy_uri)?;
    let (target_host, target_port) = uri_host_port(target_uri)?;
    let proxy_config = tungstenite_proxy_config(proxy, TungsteniteProxyScheme::Http)?;

    let tcp = connect_tcp_stream(&proxy_uri).await?;
    let proxy_tls = tls::connector()?
        .connect(tls::server_name(&proxy_host)?, tcp)
        .await
        .map_err(|e| format!("Failed to establish TLS to HTTPS proxy: {}", e))?;
    let tunneled = connect_via_proxy(proxy_tls, &proxy_config, &target_host, target_port)
        .await
        .map_err(|e| format!("Failed to establish HTTPS proxy tunnel: {}", e))?;

    Ok(Box::new(tunneled))
}

async fn connect_via_socks4_proxy(
    target_uri: &Uri,
    proxy: &Intercept,
) -> Result<BoxedCodexIo, String> {
    let proxy_uri = uri_with_resolved_port(proxy.uri())?;
    eprintln!(
        "[OpenAI Codex][websocket] SYSTEM_PROXY {}",
        proxy_display_uri(&proxy_uri)
    );

    let mut socks = SocksV4::new(proxy_uri, build_tcp_connector());
    if proxy.uri().scheme_str() == Some("socks4") {
        socks = socks.local_dns(true);
    }
    let connection = socks
        .call(target_uri.clone())
        .await
        .map_err(|e| format!("Failed to establish SOCKS4 proxy tunnel: {}", e))?;
    Ok(Box::new(connection.into_inner()))
}

async fn connect_via_socks5_proxy(
    target_uri: &Uri,
    proxy: &Intercept,
) -> Result<BoxedCodexIo, String> {
    let proxy_uri = uri_with_resolved_port(proxy.uri())?;
    eprintln!(
        "[OpenAI Codex][websocket] SYSTEM_PROXY {}",
        proxy_display_uri(&proxy_uri)
    );

    let scheme = match proxy.uri().scheme_str() {
        Some("socks5h") => TungsteniteProxyScheme::Socks5h,
        _ => TungsteniteProxyScheme::Socks5,
    };
    let (target_host, target_port) = uri_host_port(target_uri)?;
    let proxy_config = tungstenite_proxy_config(proxy, scheme)?;
    let tcp = connect_tcp_stream(&proxy_uri).await?;
    let tunneled = connect_via_proxy(tcp, &proxy_config, &target_host, target_port)
        .await
        .map_err(|e| format!("Failed to establish SOCKS5 proxy tunnel: {}", e))?;
    Ok(Box::new(tunneled))
}

pub async fn connect_websocket_transport(
    request: &http::Request<()>,
    matcher: &hyper_util::client::proxy::matcher::Matcher,
) -> Result<BoxedCodexIo, String> {
    let target_uri = websocket_proxy_match_uri(request.uri())?;

    if let Some(proxy) = matcher.intercept(&target_uri) {
        match proxy.uri().scheme_str().unwrap_or("http") {
            "http" => connect_via_http_proxy(&target_uri, &proxy).await,
            "https" => connect_via_https_proxy(&target_uri, &proxy).await,
            "socks4" | "socks4a" => connect_via_socks4_proxy(&target_uri, &proxy).await,
            "socks5" | "socks5h" => connect_via_socks5_proxy(&target_uri, &proxy).await,
            other => Err(format!(
                "Unsupported system proxy scheme for Codex websocket: {}",
                other
            )),
        }
    } else {
        Ok(Box::new(connect_tcp_stream(&target_uri).await?))
    }
}

pub async fn wrap_websocket_transport_tls(
    request: &http::Request<()>,
    stream: BoxedCodexIo,
) -> Result<BoxedCodexIo, String> {
    match request.uri().scheme_str() {
        Some("wss") => {
            let host = request
                .uri()
                .host()
                .ok_or_else(|| "Websocket endpoint is missing host".to_string())?;
            let tls_stream = tls::connector()?
                .connect(tls::server_name(host)?, stream)
                .await
                .map_err(|e| format!("Failed to establish TLS to websocket endpoint: {}", e))?;
            Ok(Box::new(tls_stream))
        }
        Some("ws") => Ok(stream),
        Some(other) => Err(format!("Unsupported websocket scheme: {}", other)),
        None => Err("Websocket endpoint is missing scheme".to_string()),
    }
}

pub fn websocket_config() -> WebSocketConfig {
    let mut extensions = ExtensionsConfig::default();
    extensions.permessage_deflate = Some(DeflateConfig::default());

    let mut config = WebSocketConfig::default();
    config.extensions = extensions;
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    #[tokio::test]
    async fn shared_connector_negotiates_deflate_and_preserves_payloads() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/responses", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket=tokio_tungstenite::accept_hdr_async_with_config(tcp,
                |request:&tokio_tungstenite::tungstenite::handshake::server::Request,response:tokio_tungstenite::tungstenite::handshake::server::Response| {
                    assert!(request.headers()["sec-websocket-extensions"].to_str().unwrap().contains("permessage-deflate"));Ok(response)
                },Some(websocket_config())).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            socket.send(message).await.unwrap();
        });
        let request = url.into_client_request().unwrap();
        let io = connect_websocket_transport(
            &request,
            &hyper_util::client::proxy::matcher::Matcher::builder().build(),
        )
        .await
        .unwrap();
        let (mut socket, response) =
            tokio_tungstenite::client_async_with_config(request, io, Some(websocket_config()))
                .await
                .unwrap();
        assert!(response.headers()["sec-websocket-extensions"]
            .to_str()
            .unwrap()
            .contains("permessage-deflate"));
        let text = "original JSON payload 中文 ".repeat(500);
        socket
            .send(Message::Text(text.clone().into()))
            .await
            .unwrap();
        assert_eq!(
            socket
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap()
                .as_str(),
            text
        );
        server.await.unwrap();
    }
}
