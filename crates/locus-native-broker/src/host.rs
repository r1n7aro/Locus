//! What a host DLL injects into the generic broker. The broker never names a
//! host: request routing, error codes, extra status fields and managed frame
//! naming all come from this configuration.

use serde_json::{Map, Value};

use crate::hang_stack_types::ManagedNamer;

/// Command types that carry managed code execution. The broker keeps these
/// requests across a client disconnect (the client reattaches by execution
/// id), answers progress polls from the published register, and signals
/// cancellation off the main thread.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionCommands {
    /// Start an execution (reattachable, long deadline).
    pub execute: &'static [&'static str],
    /// Wait for an execution that is still publishing progress.
    pub wait: &'static str,
    /// Cancel an execution (`message` = execution id, empty = all).
    pub cancel: &'static str,
    /// Read the latest progress snapshot (`message` = execution id).
    pub progress: &'static str,
}

/// One inbound request as the host hook sees it.
#[derive(Debug, Clone, Copy)]
pub struct HostRequest<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub message: &'a str,
    /// Original envelope line (trimmed).
    pub raw: &'a [u8],
}

/// A host hook's answer: `Ok(message)` or `Err(error code)`.
pub type HostReply = Result<Option<String>, String>;

/// Answer a request without the managed executor (e.g. Unity's compile result
/// register). `None` falls through to the normal routing.
pub type RequestHook = fn(&HostRequest<'_>) -> Option<HostReply>;

/// Add host fields to the status snapshot (state plane and `status_json`).
pub type StatusExtras = fn(&mut Map<String, Value>);

pub struct HostConfig {
    /// `locus_native_protocol::host_kind::*`.
    pub host_kind: &'static str,
    /// Stable id of this host process for Locus (Unity: project key,
    /// Blender: random per-process key).
    pub instance_key: String,
    /// Project path (Unity) or empty.
    pub project: String,
    /// Full `\\.\pipe\…` name.
    pub pipe_name: String,
    /// `Local\…` state plane name; empty disables the plane.
    pub state_plane_name: String,
    pub protocol_version: i32,
    /// Error answered while the host is quitting.
    pub quitting_error: &'static str,
    /// `status` answer before the host published one.
    pub default_status: &'static str,
    /// Capabilities beyond [`locus_native_protocol::capabilities::COMMON`].
    pub extra_capabilities: &'static [&'static str],
    pub execution: ExecutionCommands,
    pub request_hook: Option<RequestHook>,
    pub status_extras: Option<StatusExtras>,
    /// Runs before the broker stops (host cleanup).
    pub on_shutdown: Option<fn()>,
    /// Names managed frames in main-thread stacks (Unity: Mono).
    pub managed_namer: Option<&'static dyn ManagedNamer>,
}

impl HostConfig {
    pub fn capabilities(&self) -> Vec<&'static str> {
        locus_native_protocol::capabilities::COMMON
            .iter()
            .chain(self.extra_capabilities.iter())
            .copied()
            .collect()
    }

    pub fn is_execute(&self, kind: &str) -> bool {
        self.execution.execute.contains(&kind)
    }
}
