//! Capability identifiers the broker reports in `bridge_capabilities` and in
//! the status snapshot. Clients test for a capability instead of a version,
//! so a newer client keeps working against an older broker DLL.

pub const BROKER_V1: &str = "broker_v1";
pub const STATE_MMF_V1: &str = "broker_state_mmf_v1";
pub const QUEUE_LIMITS_V1: &str = "broker_queue_limits_v1";
pub const REQUEST_ACCEPTED_V1: &str = "broker_request_accepted_v1";
pub const MAIN_THREAD_LIVENESS_V1: &str = "broker_main_thread_liveness_v1";
pub const MAIN_THREAD_STACK_V1: &str = "broker_main_thread_stack_v1";
pub const EXECUTION_CANCEL_V1: &str = "broker_execution_cancel_v1";
pub const STACK_NAME_CACHE_V1: &str = "broker_stack_name_cache_v1";
pub const EXECUTION_PROGRESS_V1: &str = "broker_execution_progress_v1";
/// Several clients may be connected at once; responses go back to the
/// requesting connection and events reach all of them.
pub const MULTI_CLIENT_V1: &str = "broker_multi_client_v1";
/// The status snapshot carries `hostKind` and `instanceKey`.
pub const HOST_KIND_V1: &str = "broker_host_kind_v1";
/// `locus_wait_request` blocks the managed thread until a request arrives.
pub const WAIT_REQUEST_V1: &str = "broker_wait_request_v1";
/// The pipe is created with `first_pipe_instance` and a current-user DACL.
pub const PIPE_SECURITY_V1: &str = "broker_pipe_security_v1";
/// Replies are never dropped for a client that is not reading (a client too
/// far behind is disconnected), and `request_state` reports whether the
/// client's requests are still queued or running.
pub const REQUEST_STATE_V1: &str = "broker_request_state_v1";

/// Unity-only: the broker answers `get_compile_result` from a register.
pub const COMPILE_RESULT_V1: &str = "broker_compile_result_v1";

/// Capabilities every host broker reports.
pub const COMMON: &[&str] = &[
    BROKER_V1,
    STATE_MMF_V1,
    QUEUE_LIMITS_V1,
    REQUEST_ACCEPTED_V1,
    MAIN_THREAD_LIVENESS_V1,
    MAIN_THREAD_STACK_V1,
    EXECUTION_CANCEL_V1,
    STACK_NAME_CACHE_V1,
    EXECUTION_PROGRESS_V1,
    MULTI_CLIENT_V1,
    HOST_KIND_V1,
    WAIT_REQUEST_V1,
    PIPE_SECURITY_V1,
    REQUEST_STATE_V1,
];

pub fn has(capabilities: &[String], capability: &str) -> bool {
    capabilities.iter().any(|value| value == capability)
}
