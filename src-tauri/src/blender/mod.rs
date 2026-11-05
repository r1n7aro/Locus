//! Blender integration: installation table, instance hub, `.blend` header
//! parsing and the workspace service factory glue. Everything here is inert
//! while the app-level switch (`AppConfig::blender_enabled`) is off.

pub mod addon;
pub mod background;
pub mod blend_header;
pub mod detect;
pub mod exec;
pub mod hub;
pub mod installations;
pub mod manage;
pub mod process;
pub mod resources;
pub mod target;
pub mod tool;

#[cfg(test)]
mod live_tests;

use std::sync::atomic::{AtomicBool, Ordering};

/// Mirror of `AppConfig::blender_enabled` for code without a config handle.
static BLENDER_INTEGRATION_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_blender_integration_enabled(enabled: bool) {
    BLENDER_INTEGRATION_ENABLED.store(enabled, Ordering::Relaxed);
}

/// The user turned the switch (Settings > Services). Installed hosts check
/// the `disabled` marker at Blender startup and stay inert while it exists.
/// Only explicit toggles write it, so a second Locus build starting with
/// Blender off never silences the hosts another build uses. Unit tests never
/// touch the real file.
pub fn switch_changed(enabled: bool) {
    set_blender_integration_enabled(enabled);
    if !cfg!(test) {
        resources::sync_disabled_marker(enabled);
    }
}

pub fn blender_integration_enabled() -> bool {
    BLENDER_INTEGRATION_ENABLED.load(Ordering::Relaxed)
}
