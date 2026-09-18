use std::path::Path;
use std::time::{Duration, Instant};

/// Process exit can precede the final release of Windows file handles. Only
/// retry sharing/lock violations, and recheck ownership before every removal.
pub(crate) fn remove_closed_editor_marker(
    path: &Path,
    mut ensure_stopped: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        ensure_stopped()?;
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "Inspect closed Editor marker {}: {error}",
                    path.display()
                ))
            }
        };
        let mut alias = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            alias |= metadata.file_attributes() & 0x400 != 0;
        }
        if alias || !metadata.is_file() {
            return Err(format!(
                "Closed Editor marker is not an ordinary file: {}",
                path.display()
            ));
        }
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error)
                if cfg!(windows)
                    && matches!(error.raw_os_error(), Some(32 | 33))
                    && started.elapsed() < Duration::from_secs(5) =>
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(format!(
                    "Remove closed Editor marker {} after {} ms: {error}",
                    path.display(),
                    started.elapsed().as_millis()
                ))
            }
        }
    }
}

pub(crate) async fn cleanup_closed_project_markers(project: &str) -> Result<(), String> {
    let project = project.to_string();
    tokio::task::spawn_blocking(move || {
        let root = dunce::canonicalize(&project).map_err(|e| e.to_string())?;
        for relative in ["Temp/UnityLockfile", "Library/EditorInstance.json"] {
            let path = root.join(relative);
            if !path.parent().is_some_and(Path::exists) {
                continue;
            }
            let parent = dunce::canonicalize(path.parent().unwrap()).map_err(|e| e.to_string())?;
            if !parent.starts_with(&root) {
                return Err(format!(
                    "Closed Editor marker escapes its project: {}",
                    path.display()
                ));
            }
            remove_closed_editor_marker(&path, || {
                let state = super::query_current_project_editor_process_uncached(project.clone());
                if state.state != super::UnityEditorProcessState::NotRunning {
                    return Err(
                        "Editor process is not confirmed stopped; retaining its markers".into(),
                    );
                }
                Ok(())
            })?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
