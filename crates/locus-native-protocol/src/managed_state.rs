//! Managed (host-language) lifecycle states pushed into the broker. Values
//! must match the C# `ManagedState` enum and the Blender host constants.

/// Native is up; the managed executor has not registered yet (first load).
pub const INITIALIZING: i32 = 0;
/// Managed executor is registered and pumping requests.
pub const READY: i32 = 1;
/// The managed side is reloading (Unity domain reload, Blender host reload).
pub const RELOADING: i32 = 2;
/// The host process is quitting.
pub const QUITTING: i32 = 3;

pub fn name(state: i32) -> &'static str {
    match state {
        READY => "ready",
        RELOADING => "reloading",
        QUITTING => "quitting",
        _ => "initializing",
    }
}
