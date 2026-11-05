//! Replacement bytes for the Editor's `disk_apply` transaction.
//!
//! The Editor runs on this machine and reads the project directly, so each
//! replacement is a file staged under `Library/Locus/DiskApply/<transaction>/`
//! and the request carries only its path and SHA-256. The bytes never pass
//! through the bridge, whose request size is capped.
use serde_json::{json, Value};
use sha2::Digest;
use std::path::{Path, PathBuf};

const STAGING_ROOT: &str = "Library/Locus/DiskApply";

/// One replacement: the project-relative asset, the SHA-256 of the bytes it
/// must still have, and the bytes to write.
pub(crate) struct Replacement<'a> {
    pub path: String,
    pub expected_sha256: String,
    pub after: &'a [u8],
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Stages the replacements and returns their `disk_apply` entries; [`clear`]
/// removes the staged copies once the Editor answered.
pub(crate) fn entries(
    project: &Path,
    transaction_id: &str,
    replacements: &[Replacement],
) -> Result<Vec<Value>, String> {
    let dir = staging_dir(project, transaction_id)?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    let mut entries = Vec::with_capacity(replacements.len());
    for (index, replacement) in replacements.iter().enumerate() {
        let name = format!("{index}.bytes");
        let file = dir.join(&name);
        std::fs::write(&file, replacement.after)
            .map_err(|error| format!("Could not stage {}: {error}", file.display()))?;
        entries.push(json!({"path":replacement.path,"expected_sha256":replacement.expected_sha256,
            "staged_path":format!("{STAGING_ROOT}/{transaction_id}/{name}"),
            "after_sha256":sha256_hex(replacement.after)}));
    }
    Ok(entries)
}

/// Staged copies are never needed afterwards: every caller journals its own
/// before/after bytes. A copy left by a crash only occupies `Library`.
pub(crate) fn clear(project: &Path, transaction_id: &str) {
    if let Ok(dir) = staging_dir(project, transaction_id) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn staging_dir(project: &Path, transaction_id: &str) -> Result<PathBuf, String> {
    let id = uuid::Uuid::parse_str(transaction_id)
        .map_err(|_| "disk_apply transaction IDs must be UUIDs".to_string())?;
    Ok(project.join(STAGING_ROOT).join(id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_carry_paths_and_hashes_of_staged_bytes() {
        let project = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        // Larger than the bridge's 16 MiB request limit.
        let bytes = vec![b'x'; 20 * 1024 * 1024];
        let replacement = Replacement { path: "Assets/Big.unity".into(), expected_sha256: "0".repeat(64), after: &bytes };
        let entries = entries(project.path(), &id, &[replacement]).unwrap();
        assert_eq!(entries[0]["staged_path"], format!("Library/Locus/DiskApply/{id}/0.bytes"));
        assert_eq!(entries[0]["after_sha256"], sha256_hex(&bytes));
        let staged = project.path().join(format!("Library/Locus/DiskApply/{id}/0.bytes"));
        assert_eq!(std::fs::read(&staged).unwrap(), bytes);
        assert!(serde_json::to_vec(&entries).unwrap().len() < 1024);
        clear(project.path(), &id);
        assert!(!staged.parent().unwrap().exists());
    }

    #[test]
    fn staging_rejects_transaction_ids_that_are_not_uuids() {
        let project = tempfile::tempdir().unwrap();
        assert!(entries(project.path(), "../escape", &[]).is_err());
    }
}
