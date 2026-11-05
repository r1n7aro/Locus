//! Names of the per-host kernel objects (pipe, state plane). The Unity names
//! are derived from the project path and must stay byte-identical to what
//! older plugins and Locus builds compute; Blender names come from a random
//! per-process instance key.

use sha2::{Digest, Sha256};

fn strip_extended_path_prefix(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

/// Lowercase, backslash-separated, no trailing separator (drive roots keep
/// theirs). Mirrors `LocusBridge.GenerateNativePipeName` on the Unity side.
pub fn normalize_project_path(project_path: &str) -> String {
    let trimmed = strip_extended_path_prefix(project_path).trim();
    let mut value = trimmed.replace('/', "\\");
    while value.ends_with('\\') && value.len() > 3 {
        value.pop();
    }
    value.to_ascii_lowercase()
}

/// First 16 bytes of SHA-256 over the normalized project path, as hex.
pub fn project_key(project_path: &str) -> String {
    let normalized = normalize_project_path(project_path);
    let digest = Sha256::digest(normalized.as_bytes());
    let mut out = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// Unity state plane: `Local\LocusNativeBrokerState_<project key>`.
pub fn unity_state_plane_name(project_path: &str) -> String {
    format!(r"Local\LocusNativeBrokerState_{}", project_key(project_path))
}

/// Unity broker pipe name without the `\\.\pipe\` prefix.
pub fn unity_pipe_name_part(project_path: &str) -> String {
    format!("locus_unity_native_{}", project_key(project_path))
}

/// Blender state plane: `Local\LocusBlenderState_<instance key>`.
pub fn blender_state_plane_name(instance_key: &str) -> String {
    format!(r"Local\LocusBlenderState_{}", sanitize_key(instance_key))
}

/// Blender broker pipe name without the `\\.\pipe\` prefix.
pub fn blender_pipe_name_part(instance_key: &str) -> String {
    format!("locus_blender_{}", sanitize_key(instance_key))
}

/// Instance keys become part of kernel object names; keep them to a safe
/// alphabet and length.
pub fn sanitize_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(64)
        .collect()
}

/// Full `\\.\pipe\…` path for a bare or full pipe name.
pub fn normalize_pipe_name(pipe_name: &str) -> String {
    let value = pipe_name.trim();
    if value.starts_with(r"\\.\pipe\") {
        value.to_string()
    } else {
        format!(r"\\.\pipe\{}", value.trim_start_matches('\\'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_names_match_the_historical_derivation() {
        // Pinned: older plugins and Locus builds compute exactly this.
        let key = project_key("F:/Projects/Game/");
        assert_eq!(key.len(), 32);
        assert_eq!(key, project_key(r"f:\projects\game"));
        assert_eq!(key, project_key(r"\\?\F:\Projects\Game"));
        assert_eq!(
            unity_state_plane_name("F:/Projects/Game"),
            format!(r"Local\LocusNativeBrokerState_{key}")
        );
        assert_eq!(
            unity_pipe_name_part("F:/Projects/Game"),
            format!("locus_unity_native_{key}")
        );
        assert_eq!(normalize_project_path("C:\\"), "c:\\");
    }

    #[test]
    fn blender_names_use_a_sanitized_instance_key() {
        assert_eq!(
            blender_pipe_name_part("1234_ab-CD/../x"),
            "locus_blender_1234_ab-CDx"
        );
        assert_eq!(
            blender_state_plane_name("k1"),
            r"Local\LocusBlenderState_k1"
        );
        assert_eq!(normalize_pipe_name("locus_blender_k1"), r"\\.\pipe\locus_blender_k1");
        assert_eq!(normalize_pipe_name(r"\\.\pipe\x"), r"\\.\pipe\x");
    }
}
