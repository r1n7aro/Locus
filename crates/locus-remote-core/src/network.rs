//! Shared network options; desktop proxy selection stays an injected host concern.
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodexHttpProfile {
    Authentication,
    Responses,
    Compact,
    Models,
    Usage,
    ConsumeCredit,
    Web,
}

pub const MODELS_TIMEOUT_SECS: u64 = 5;
pub const USAGE_TIMEOUT_SECS: u64 = 8;
pub const CONSUME_TIMEOUT_SECS: u64 = 10;
pub const COMPACT_TIMEOUT: Duration = Duration::from_secs(300);
pub const UPSTREAM_ERROR_MARKER: &str = "[locus-upstream] ";
pub fn upstream_auth_failure(status: http::StatusCode, headers: &http::HeaderMap) -> bool {
    status == http::StatusCode::UNAUTHORIZED
        && headers
            .get("x-locus-error-origin")
            .is_some_and(|value| value == "upstream")
}

/// Defaults are the desktop defaults. Callers inject only their proxy route.
pub fn codex_http_options(profile: CodexHttpProfile) -> ReqwestClientOptions {
    let options = ReqwestClientOptions::new().gzip(true).deflate(true);
    match profile {
        CodexHttpProfile::Authentication => options,
        CodexHttpProfile::Responses => options
            .tcp_keepalive(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(30)),
        CodexHttpProfile::Compact => {
            codex_http_options(CodexHttpProfile::Responses).timeout(COMPACT_TIMEOUT)
        }
        CodexHttpProfile::Models => options
            .connect_timeout(Duration::from_secs(MODELS_TIMEOUT_SECS))
            .timeout(Duration::from_secs(MODELS_TIMEOUT_SECS)),
        CodexHttpProfile::Usage => options
            .connect_timeout(Duration::from_secs(USAGE_TIMEOUT_SECS))
            .timeout(Duration::from_secs(USAGE_TIMEOUT_SECS)),
        CodexHttpProfile::ConsumeCredit => options
            .connect_timeout(Duration::from_secs(CONSUME_TIMEOUT_SECS))
            .timeout(Duration::from_secs(CONSUME_TIMEOUT_SECS)),
        CodexHttpProfile::Web => options
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(300)),
    }
}

pub fn codex_http_client(profile: CodexHttpProfile) -> Result<reqwest::Client, reqwest::Error> {
    codex_http_options(profile)
        .apply(reqwest::Client::builder().tls_backend_rustls())
        .build()
}

#[derive(Debug, Clone)]
pub struct ReqwestClientOptions {
    connect_timeout: Option<Duration>,
    timeout: Option<Duration>,
    tcp_keepalive: Option<Duration>,
    tcp_nodelay: Option<bool>,
    pool_idle_timeout: Option<Duration>,
    pool_max_idle_per_host: Option<usize>,
    http2_adaptive_window: Option<bool>,
    http2_keep_alive_interval: Option<Duration>,
    http2_keep_alive_timeout: Option<Duration>,
    user_agent: Option<String>,
    gzip: Option<bool>,
    deflate: Option<bool>,
    follow_redirects: Option<bool>,
}

impl Default for ReqwestClientOptions {
    fn default() -> Self {
        Self {
            connect_timeout: None,
            timeout: None,
            tcp_keepalive: None,
            tcp_nodelay: None,
            pool_idle_timeout: None,
            pool_max_idle_per_host: None,
            http2_adaptive_window: None,
            http2_keep_alive_interval: None,
            http2_keep_alive_timeout: None,
            user_agent: None,
            gzip: None,
            deflate: None,
            follow_redirects: None,
        }
    }
}

impl ReqwestClientOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn connect_timeout(mut self, value: Duration) -> Self {
        self.connect_timeout = Some(value);
        self
    }

    pub fn timeout(mut self, value: Duration) -> Self {
        self.timeout = Some(value);
        self
    }

    pub fn tcp_keepalive(mut self, value: Duration) -> Self {
        self.tcp_keepalive = Some(value);
        self
    }

    pub fn tcp_nodelay(mut self, value: bool) -> Self {
        self.tcp_nodelay = Some(value);
        self
    }

    pub fn pool_idle_timeout(mut self, value: Duration) -> Self {
        self.pool_idle_timeout = Some(value);
        self
    }

    pub fn pool_max_idle_per_host(mut self, value: usize) -> Self {
        self.pool_max_idle_per_host = Some(value);
        self
    }

    pub fn http2_adaptive_window(mut self, value: bool) -> Self {
        self.http2_adaptive_window = Some(value);
        self
    }

    pub fn http2_keep_alive_interval(mut self, value: Duration) -> Self {
        self.http2_keep_alive_interval = Some(value);
        self
    }

    pub fn http2_keep_alive_timeout(mut self, value: Duration) -> Self {
        self.http2_keep_alive_timeout = Some(value);
        self
    }

    pub fn user_agent(mut self, value: impl Into<String>) -> Self {
        self.user_agent = Some(value.into());
        self
    }

    pub fn gzip(mut self, value: bool) -> Self {
        self.gzip = Some(value);
        self
    }

    pub fn deflate(mut self, value: bool) -> Self {
        self.deflate = Some(value);
        self
    }

    /// `false` returns 3xx responses to the caller instead of following them,
    /// so it can inspect every hop (e.g. a destination check per redirect).
    pub fn follow_redirects(mut self, value: bool) -> Self {
        self.follow_redirects = Some(value);
        self
    }
}

impl ReqwestClientOptions {
    pub fn apply(self, mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        if let Some(value) = self.connect_timeout {
            builder = builder.connect_timeout(value);
        }
        if let Some(value) = self.timeout {
            builder = builder.timeout(value);
        }
        if let Some(value) = self.tcp_keepalive {
            builder = builder.tcp_keepalive(value);
        }
        if let Some(value) = self.tcp_nodelay {
            builder = builder.tcp_nodelay(value);
        }
        if let Some(value) = self.pool_idle_timeout {
            builder = builder.pool_idle_timeout(value);
        }
        if let Some(value) = self.pool_max_idle_per_host {
            builder = builder.pool_max_idle_per_host(value);
        }
        if let Some(value) = self.http2_adaptive_window {
            builder = builder.http2_adaptive_window(value);
        }
        if let Some(value) = self.http2_keep_alive_interval {
            builder = builder.http2_keep_alive_interval(value);
        }
        if let Some(value) = self.http2_keep_alive_timeout {
            builder = builder.http2_keep_alive_timeout(value);
        }
        if let Some(value) = self.user_agent {
            builder = builder.user_agent(value);
        }
        if let Some(value) = self.gzip {
            builder = builder.gzip(value);
        }
        if let Some(value) = self.deflate {
            builder = builder.deflate(value);
        }
        if self.follow_redirects == Some(false) {
            builder = builder.redirect(reqwest::redirect::Policy::none());
        }

        builder
    }
}
