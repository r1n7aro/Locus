//! The bundled Blender host sources (`locus_blender/`): add-on package,
//! bootstrap for background instances, installation probe.

use std::path::PathBuf;
use std::sync::OnceLock;

/// `locus_blender/` next to the executable (release) or in the repository
/// (development builds).
pub fn host_source_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let mut candidates = Vec::new();
        #[cfg(debug_assertions)]
        candidates.extend([PathBuf::from("../locus_blender"), PathBuf::from("locus_blender")]);
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                #[cfg(debug_assertions)]
                candidates.push(exe_dir.join("../../../locus_blender"));
                candidates.push(exe_dir.join("locus_blender"));
            }
        }
        let found = candidates
            .into_iter()
            .find(|dir| dir.join("locus_host").join("__init__.py").is_file())
            .map(|dir| dunce::canonicalize(&dir).unwrap_or(dir));
        if found.is_none() {
            eprintln!("[Locus] Blender host sources (locus_blender) not found");
        }
        found
    })
    .clone()
}

pub fn host_package_dir() -> Option<PathBuf> {
    host_source_dir().map(|dir| dir.join("locus_host"))
}

pub fn probe_script() -> Option<PathBuf> {
    host_source_dir().map(|dir| dir.join("probe.py")).filter(|path| path.is_file())
}

pub fn bootstrap_script() -> Option<PathBuf> {
    host_source_dir().map(|dir| dir.join("bootstrap.py")).filter(|path| path.is_file())
}

/// Directory where Locus keeps Blender state shared with the host add-on
/// (`instances/` registry, `disabled` marker). Overridable for isolated tests.
pub fn locus_blender_data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Locus").join("blender")
}

pub fn registry_dir() -> PathBuf {
    std::env::var_os("LOCUS_BLENDER_REGISTRY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| locus_blender_data_dir().join("instances"))
}

pub fn disabled_marker() -> PathBuf {
    locus_blender_data_dir().join("disabled")
}

/// Keep the add-on's startup check in step with the app switch: present while
/// Blender is off, so Blender processes started later stay inert.
pub fn sync_disabled_marker(enabled: bool) {
    let marker = disabled_marker();
    if enabled {
        let _ = std::fs::remove_file(&marker);
    } else if locus_blender_data_dir().is_dir() {
        // Without the data directory Blender was never used with Locus and
        // there is no host to silence.
        if let Some(parent) = marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&marker, b"Blender integration is turned off in Locus\n");
    }
}
