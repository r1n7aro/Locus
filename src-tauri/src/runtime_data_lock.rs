use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::fs_std::FileExt;

const LOCK_FILE_NAME: &str = ".locus-instance.lock";

pub(crate) struct RuntimeDataDirLock {
    _file: File,
    path: PathBuf,
}

impl RuntimeDataDirLock {
    pub(crate) fn acquire(data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|error| {
            format!(
                "Failed to create runtime data directory '{}': {}",
                data_dir.display(),
                error
            )
        })?;
        let path = data_dir.join(LOCK_FILE_NAME);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "Failed to open runtime data directory lock '{}': {}",
                    path.display(),
                    error
                )
            })?;

        match file.try_lock_exclusive() {
            Ok(true) => {}
            Ok(false) => {
                let owner = std::fs::read_to_string(&path)
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "owner metadata unavailable".to_string());
                return Err(format!(
                    "Locus data directory '{}' is already in use by another process ({owner}). Close that Locus instance or use an isolated LOCUS_RUNTIME_DATA_DIR.",
                    data_dir.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "Failed to lock runtime data directory '{}': {}",
                    data_dir.display(),
                    error
                ));
            }
        }

        file.set_len(0).map_err(|error| {
            format!(
                "Failed to reset runtime data directory lock '{}': {}",
                path.display(),
                error
            )
        })?;
        file.seek(SeekFrom::Start(0)).map_err(|error| {
            format!(
                "Failed to seek runtime data directory lock '{}': {}",
                path.display(),
                error
            )
        })?;
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let executable = std::env::current_exe()
            .map(|value| value.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        write!(
            file,
            "pid={} started_at={} executable={}",
            std::process::id(),
            started_at,
            executable
        )
        .map_err(|error| {
            format!(
                "Failed to write runtime data directory lock '{}': {}",
                path.display(),
                error
            )
        })?;
        file.flush().map_err(|error| {
            format!(
                "Failed to flush runtime data directory lock '{}': {}",
                path.display(),
                error
            )
        })?;

        Ok(Self { _file: file, path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeDataDirLock;

    #[test]
    fn exclusive_lock_blocks_a_second_runtime_and_recovers_after_drop() {
        let root = tempfile::tempdir().expect("tempdir");
        let first = RuntimeDataDirLock::acquire(root.path()).expect("first lock");
        let error = RuntimeDataDirLock::acquire(root.path())
            .err()
            .expect("second lock should fail");

        assert!(error.contains("already in use by another process"));
        assert!(first.path().is_file());

        drop(first);
        RuntimeDataDirLock::acquire(root.path()).expect("lock after release");
    }
}
