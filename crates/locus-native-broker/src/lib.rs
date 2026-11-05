//! Host-agnostic in-process native broker.
//!
//! Locus talks to editor processes (Unity, Blender) over a Windows named pipe
//! served from inside the host process. The managed layer of the host (Unity
//! C#, Blender Python) does all host-API work on its main thread; this broker
//! owns the long-lived communication substrate on its own thread:
//!
//!   * the pipe server (several clients at once; a client that stops reading
//!     keeps every reply but only its latest events),
//!   * the request queue the managed executor polls (or blocks on),
//!   * answers to `ping`, `status`, `bridge_capabilities` and
//!     `main_thread_stack` without the main thread,
//!   * managed lifecycle tracking, main-thread heartbeat observation,
//!     execution progress and cancellation registers,
//!   * the state plane (shared-memory status snapshot).
//!
//! It never calls host internals, holds no host or interpreter lock, and does
//! no work on the host main thread beyond the FFI calls the host makes.
//! Host specifics come from [`host::HostConfig`]. The crate exports no C
//! symbols: each host DLL invokes [`export_common_ffi!`] (and optionally
//! [`export_host_ffi!`]) once.

pub use locus_native_protocol as protocol;

pub mod api;
pub mod ffi;
pub mod hang_stack_types;
pub mod heartbeat;
pub mod host;

#[cfg(windows)]
pub mod broker;
#[cfg(windows)]
pub mod hang_stack;
#[cfg(windows)]
mod outbox;
#[cfg(windows)]
mod pipe_security;
#[cfg(windows)]
mod server;
#[cfg(windows)]
mod state_plane;

#[cfg(all(test, windows))]
mod tests;

/// Normalize a pipe name to the full `\\.\pipe\…` path.
pub fn normalize_pipe_name(pipe_name: &str) -> String {
    protocol::names::normalize_pipe_name(pipe_name)
}
