//! Error codes the broker answers itself (`ok: false`, `error: <code>`).
//! Host-specific codes (for example the quitting code) come from the host
//! configuration.

pub const MANAGED_RELOADING: &str = "managed_reloading";
pub const MANAGED_NOT_READY: &str = "managed_not_ready";
pub const QUEUE_FULL: &str = "native_queue_full";
pub const INFLIGHT_FULL: &str = "native_inflight_full";
pub const PAYLOAD_TOO_LARGE: &str = "native_payload_too_large";
pub const REQUEST_TIMED_OUT: &str = "native_request_timed_out";
pub const MAIN_THREAD_UNRESPONSIVE: &str = "native_main_thread_unresponsive";
pub const RELOAD_INTERRUPTED: &str = "domain_reload_interrupted";
pub const STACK_FAILED: &str = "main_thread_stack_failed";
pub const INVALID_REQUEST: &str = "native_invalid_request";

/// Unity reports quitting with this code (kept for existing clients).
pub const UNITY_PROCESS_EXITING: &str = "unity_process_exiting";
pub const BLENDER_PROCESS_EXITING: &str = "blender_process_exiting";

/// The request never reached the host's executor, so retrying it is safe.
pub fn is_not_executed(code: &str) -> bool {
    matches!(
        code,
        MANAGED_RELOADING
            | MANAGED_NOT_READY
            | QUEUE_FULL
            | INFLIGHT_FULL
            | PAYLOAD_TOO_LARGE
            | MAIN_THREAD_UNRESPONSIVE
            | UNITY_PROCESS_EXITING
            | BLENDER_PROCESS_EXITING
    )
}
