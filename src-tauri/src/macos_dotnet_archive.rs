//! Strict extraction for the macOS .NET runtime archive.
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path};

pub(super) fn extract_runtime(archive: &Path, destination: &Path) -> Result<(), String> {
    let input = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(input));
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let relative = entry.path().map_err(|e| e.to_string())?.into_owned();
        if relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(format!(
                "Unsafe runtime archive path: {}",
                relative.display()
            ));
        }
        let kind = entry.header().entry_type();
        if !kind.is_dir() && !kind.is_file() {
            return Err(format!(
                "Unsupported runtime archive entry: {}",
                relative.display()
            ));
        }
        if !entry.unpack_in(destination).map_err(|e| e.to_string())? {
            return Err("Runtime archive entry escaped its install directory".into());
        }
    }
    let host = destination.join("dotnet");
    if !host.is_file() {
        return Err("Runtime archive did not contain the dotnet host".into());
    }
    #[cfg(unix)]
    std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("Failed to make dotnet executable: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};

    fn archive(root: &Path, name: &str, kind: tar::EntryType) -> std::path::PathBuf {
        let path = root.join("runtime.tar.gz");
        let output = std::fs::File::create(&path).unwrap();
        let mut builder = tar::Builder::new(GzEncoder::new(output, Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_mode(0o644);
        header.set_size(0);
        // Raw header permits testing traversal rejected by Builder::append_path.
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        if kind.is_symlink() || kind.is_hard_link() {
            header.set_link_name("../outside").unwrap();
        }
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        path
    }

    #[test]
    fn extracts_host_and_restores_executable_permissions() {
        let root = tempfile::tempdir().unwrap();
        let archive = archive(root.path(), "dotnet", tar::EntryType::Regular);
        let destination = root.path().join("runtime");
        std::fs::create_dir(&destination).unwrap();
        extract_runtime(&archive, &destination).unwrap();
        assert!(destination.join("dotnet").is_file());
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(destination.join("dotnet"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
    }

    #[test]
    fn rejects_unsafe_archive_paths_before_writing_outside_runtime() {
        for name in ["../outside", "/outside"] {
            let root = tempfile::tempdir().unwrap();
            let archive = archive(root.path(), name, tar::EntryType::Regular);
            let destination = root.path().join("runtime");
            std::fs::create_dir(&destination).unwrap();
            assert!(extract_runtime(&archive, &destination)
                .unwrap_err()
                .contains("Unsafe runtime archive path"));
            assert!(!root.path().join("outside").exists());
        }
    }

    #[test]
    fn rejects_links_and_incomplete_archives() {
        for kind in [
            tar::EntryType::Symlink,
            tar::EntryType::Link,
            tar::EntryType::Regular,
        ] {
            let root = tempfile::tempdir().unwrap();
            let archive = archive(root.path(), "other", kind);
            let destination = root.path().join("runtime");
            std::fs::create_dir(&destination).unwrap();
            assert!(extract_runtime(&archive, &destination).is_err());
            assert!(!destination.join(".locus-complete").exists());
        }
    }

    #[test]
    fn optionally_validates_official_downloads() {
        let Some(directory) = std::env::var_os("LOCUS_TEST_DOTNET_ARCHIVES") else {
            return;
        };
        for rid in ["osx-arm64", "osx-x64"] {
            let root = tempfile::tempdir().unwrap();
            extract_runtime(
                &Path::new(&directory).join(format!("{rid}.tar.gz")),
                root.path(),
            )
            .unwrap();
            assert!(root.path().join("host/fxr").is_dir());
            assert!(root.path().join("shared/Microsoft.NETCore.App").is_dir());
        }
    }
}
