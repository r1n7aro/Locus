use serde::{Deserialize, Serialize};
use tauri::Emitter;

// ── Severity ──

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ErrorSeverity {
    Error,
    Warning,
    Info,
}

// ── AppError ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    pub retryable: bool,
    pub severity: ErrorSeverity,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
            operation: None,
            retryable: false,
            severity: ErrorSeverity::Error,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn operation(mut self, op: impl Into<String>) -> Self {
        self.operation = Some(op.into());
        self
    }

    pub fn retryable(mut self, r: bool) -> Self {
        self.retryable = r;
        self
    }

    pub fn severity(mut self, s: ErrorSeverity) -> Self {
        self.severity = s;
        self
    }

    /// Emit as `"app-error"` event for background failures with no request/response boundary.
    pub fn emit_background(app_handle: &tauri::AppHandle, error: &AppError) {
        if let Err(e) = app_handle.emit("app-error", error) {
            eprintln!(
                "[Locus] failed to emit app-error event: {} (original: {})",
                e, error
            );
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

// ── Migration bridge: From<String> / From<&str> ──
// Allows changing command signatures from Result<T, String> to Result<T, AppError>
// without modifying every error site at once.

impl From<String> for AppError {
    fn from(s: String) -> Self {
        Self::new("legacy.string_error", &s).detail(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        Self::new("legacy.string_error", s)
    }
}

// ── From<anyhow::Error> ──

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        let message = format!("{}", err);
        let detail = format!("{:#}", err);
        Self::new("internal.unknown", message).detail(detail)
    }
}

// ── IntoAppError trait ──
// Used at command boundaries to map anyhow errors to domain-specific AppError.

pub trait IntoAppError {
    fn app_err(self, code: &str, operation: &str) -> AppError;
}

impl IntoAppError for anyhow::Error {
    fn app_err(self, code: &str, operation: &str) -> AppError {
        let message = format!("{}", self);
        let detail = format!("{:#}", self);
        AppError::new(code, message)
            .detail(detail)
            .operation(operation.to_string())
    }
}

/// Convenience type alias for command return types.
pub type AppResult<T> = Result<T, AppError>;
