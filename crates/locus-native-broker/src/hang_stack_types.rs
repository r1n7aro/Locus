//! Platform-neutral part of the main-thread stack capture: the wire model and
//! the managed-frame naming hook a host can provide.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CaptureRequest {
    pub samples: Option<u32>,
    pub interval_ms: Option<u64>,
    pub max_frames: Option<usize>,
    pub resolve_managed: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    pub version: u32,
    pub thread_id: u32,
    pub process_id: u32,
    pub started_at_ms: i64,
    pub wall_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_start_100ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_end_100ns: Option<u64>,
    pub samples: Vec<SampleOut>,
    pub managed: Vec<ManagedFrame>,
    /// `ok` | `skipped` | `unavailable` | `timeout` | `blocked` | `error`.
    pub managed_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleOut {
    pub captured_at_ms: i64,
    pub suspend_us: u64,
    pub stack_bytes: usize,
    pub frames: Vec<FrameOut>,
    /// Why the walk stopped before the thread's root frame, if it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unwind_stop: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameOut {
    pub ip: u64,
    pub sp: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedFrame {
    pub ip: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assembly: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub native_offset: u64,
}

/// Names JIT frames of a managed runtime embedded in the host (Unity: Mono).
/// The broker caches names per managed generation, runs [`Self::name`] on a
/// dedicated helper thread with a deadline, and never calls it while a
/// previous lookup is still stuck.
pub trait ManagedNamer: Send + Sync {
    /// Resolve runtime entry points on the thread that initializes the broker
    /// (the host main thread), never later during a hang.
    fn prepare(&self) {}

    /// `None` when the runtime is present; otherwise why naming is unavailable.
    fn unavailable_reason(&self) -> Option<&'static str>;

    /// Name `(frame ip, lookup address)` pairs. Frames the runtime does not
    /// own are simply absent from the result.
    ///
    /// # Safety
    /// Runs on a dedicated helper thread that may attach to the runtime.
    unsafe fn name(&self, lookups: &[(u64, u64)]) -> Result<Vec<ManagedFrame>, String>;
}
