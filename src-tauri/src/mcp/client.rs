//! Transport-agnostic MCP client handle.
//!
//! The manager and the handshake helpers speak to this enum; stdio and
//! Streamable HTTP differ only in how messages move, not in what the
//! lifecycle looks like (spawn/connect → handshake → requests → dead →
//! lazy reconnect).

use std::time::Duration;

use serde_json::Value;

use super::config::{McpServerConfig, McpTransport};
use super::http::HttpMcpClient;
use super::stdio::StdioMcpClient;

pub enum McpClient {
    Stdio(StdioMcpClient),
    Http(HttpMcpClient),
}

impl McpClient {
    pub async fn connect(config: &McpServerConfig) -> Result<Self, String> {
        match config.transport {
            McpTransport::Stdio => Ok(Self::Stdio(StdioMcpClient::spawn(config).await?)),
            McpTransport::Http => Ok(Self::Http(HttpMcpClient::connect(config)?)),
        }
    }

    /// True when the connection can no longer serve requests (process died /
    /// session unrecoverable) and the manager should lazily reconnect.
    pub fn is_dead(&self) -> bool {
        match self {
            Self::Stdio(client) => client.has_exited(),
            Self::Http(client) => client.is_dead(),
        }
    }

    pub async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        match self {
            Self::Stdio(client) => client.request(method, params, timeout).await,
            Self::Http(client) => client.request(method, params, timeout).await,
        }
    }

    pub async fn request_cancellable(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        cancel: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> Result<Value, String> {
        match self {
            Self::Stdio(client) => {
                client
                    .request_cancellable(method, params, timeout, cancel)
                    .await
            }
            Self::Http(client) => {
                client
                    .request_cancellable(method, params, timeout, cancel)
                    .await
            }
        }
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), String> {
        match self {
            Self::Stdio(client) => client.notify(method, params).await,
            Self::Http(client) => client.notify(method, params).await,
        }
    }

    pub async fn shutdown(&self) {
        match self {
            Self::Stdio(client) => client.shutdown().await,
            Self::Http(client) => client.shutdown().await,
        }
    }

    pub fn kill_for_exit(&self) {
        match self {
            Self::Stdio(client) => client.kill_for_exit(),
            Self::Http(client) => client.kill_for_exit(),
        }
    }

    /// Tears down a connection that stopped answering pings. Unlike
    /// shutdown()/kill_for_exit() this does NOT mark the exit as expected,
    /// so an auto-restart watcher treats it as a crash and restarts.
    pub fn abandon_unresponsive(&self) {
        match self {
            Self::Stdio(client) => client.kill_unresponsive(),
            Self::Http(client) => client.mark_dead(),
        }
    }

    pub fn as_stdio(&self) -> Option<&StdioMcpClient> {
        match self {
            Self::Stdio(client) => Some(client),
            Self::Http(_) => None,
        }
    }
}
