//! Protocol shared by the in-process native brokers (`locus_unity_native`,
//! `locus_blender_native`) and the Locus desktop app.
//!
//! The brokers serve a Windows named pipe carrying newline-delimited JSON
//! envelopes and publish their status into a named shared-memory "state
//! plane". Everything both sides must agree on lives here: the envelope, the
//! state-plane byte layout, object names, managed lifecycle states, capability
//! identifiers and broker error codes. The crate is platform independent; the
//! Win32 mapping and pipe code stay with the broker and the client.

pub mod capabilities;
pub mod envelope;
pub mod errors;
pub mod managed_state;
pub mod names;
pub mod state_plane;

/// Protocol version the managed hosts and the broker agree on.
pub const NATIVE_PROTOCOL_VERSION: i32 = 1;

/// Hosts that embed a native broker. Sent in `locus_init_host` and published
/// in the status snapshot (`hostKind`).
pub mod host_kind {
    pub const UNITY: &str = "unity";
    pub const BLENDER: &str = "blender";
}
