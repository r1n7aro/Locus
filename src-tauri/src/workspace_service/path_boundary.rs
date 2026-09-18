//! Comparison-only path boundaries. Never use these normalized components as
//! persisted checkout identities or as rewritten Git/Editor arguments.
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
enum Part {
    Root,
    Normal(OsString),
    #[cfg(windows)]
    Drive(u8),
    #[cfg(windows)]
    Unc(OsString, OsString),
}

fn names_equal(left: &OsStr, right: &OsStr) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn CompareStringOrdinal(
                left: *const u16,
                left_len: i32,
                right: *const u16,
                right_len: i32,
                ignore_case: i32,
            ) -> i32;
        }
        let left: Vec<_> = left.encode_wide().collect();
        let right: Vec<_> = right.encode_wide().collect();
        let (Ok(left_len), Ok(right_len)) = (i32::try_from(left.len()), i32::try_from(right.len()))
        else {
            return false;
        };
        // Use the filesystem's ordinal comparison; never lowercase lossy UTF-8
        // or collapse two distinct non-Unicode Windows component spellings.
        unsafe { CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) == 2 }
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn parts(path: &Path) -> Result<Vec<Part>, String> {
    if !path.is_absolute() {
        return Err(format!(
            "Path boundary requires an absolute path: {}",
            path.display()
        ));
    }
    let mut result = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => result.push(Part::Root),
            Component::Normal(name) => result.push(Part::Normal(name.into())),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("Path boundary does not accept parent traversal".into())
            }
            #[cfg(windows)]
            Component::Prefix(prefix) => {
                use std::path::Prefix;
                match prefix.kind() {
                    Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                        result.push(Part::Drive(drive.to_ascii_uppercase()))
                    }
                    Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                        result.push(Part::Unc(server.into(), share.into()))
                    }
                    _ => return Err("Path boundary does not accept device namespaces".into()),
                }
            }
            #[cfg(not(windows))]
            Component::Prefix(_) => return Err("Unsupported path prefix".into()),
        }
    }
    Ok(result)
}
fn part_equal(left: &Part, right: &Part) -> bool {
    match (left, right) {
        (Part::Root, Part::Root) => true,
        (Part::Normal(left), Part::Normal(right)) => names_equal(left, right),
        #[cfg(windows)]
        (Part::Drive(left), Part::Drive(right)) => left == right,
        #[cfg(windows)]
        (Part::Unc(ls, lp), Part::Unc(rs, rp)) => names_equal(ls, rs) && names_equal(lp, rp),
        _ => false,
    }
}

/// Lexical comparison only: equivalent Windows prefix spellings are accepted,
/// but filesystem aliases/reparse targets are not resolved or authorized here.
pub(crate) fn path_relative(root: &Path, child: &Path) -> Result<Option<PathBuf>, String> {
    let root = parts(root)?;
    let child = parts(child)?;
    if root.len() > child.len() || !root.iter().zip(&child).all(|(a, b)| part_equal(a, b)) {
        return Ok(None);
    }
    let mut relative = PathBuf::new();
    for part in &child[root.len()..] {
        match part {
            Part::Normal(name) => relative.push(name),
            _ => return Err("Unexpected root inside path boundary".into()),
        }
    }
    Ok(Some(relative))
}
pub(crate) fn path_components_equal(left: &Path, right: &Path) -> Result<bool, String> {
    Ok(path_relative(left, right)?.is_some_and(|relative| relative.as_os_str().is_empty()))
}

/// Canonicalize both existing paths using the same native representation before
/// comparing. Following a reparse point outside root yields None, not a bypass.
pub(crate) fn existing_path_relative(root: &Path, child: &Path) -> Result<Option<PathBuf>, String> {
    let root = std::fs::canonicalize(root).map_err(|error| error.to_string())?;
    let child = std::fs::canonicalize(child).map_err(|error| error.to_string())?;
    path_relative(&root, &child)
}
pub(crate) fn existing_path_contains(root: &Path, child: &Path) -> Result<bool, String> {
    Ok(existing_path_relative(root, child)?.is_some())
}

fn resolve_existing_ancestor(path: &Path) -> Result<PathBuf, String> {
    // Validate before walking; an absent `..` segment must not become an alias.
    parts(path)?;
    let mut parent = path;
    let mut suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(parent) {
            Ok(_) => {
                let mut resolved =
                    std::fs::canonicalize(parent).map_err(|error| error.to_string())?;
                for name in suffix.into_iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    parent
                        .file_name()
                        .ok_or("Missing absolute path ancestor")?
                        .to_os_string(),
                );
                parent = parent.parent().ok_or("Missing absolute path parent")?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// For validated pending/reservation paths whose final directory may not yet
/// exist. Resolve the actual existing ancestor on both sides, retain the exact
/// missing component suffix, and compare components. Existing aliases remain
/// subject to physical containment; no lexical `..` or device path is accepted.
pub(crate) fn prospective_path_relative(
    root: &Path,
    child: &Path,
) -> Result<Option<PathBuf>, String> {
    path_relative(
        &resolve_existing_ancestor(root)?,
        &resolve_existing_ancestor(child)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "locus-boundary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn real_long_child_and_future_descendants_obey_component_boundaries() {
        let base = base();
        let root = base.join("project");
        std::fs::create_dir(&root).unwrap();
        let mut child = root.clone();
        while child.as_os_str().len() < 300 {
            child.push("long-asset-component-1234567890");
        }
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("asset.asset"), b"proof").unwrap();
        let child = child.join("asset.asset");
        assert!(existing_path_contains(&root, &child).unwrap());
        assert!(existing_path_relative(&root, &child)
            .unwrap()
            .unwrap()
            .ends_with("asset.asset"));
        let sibling = base.join("project-copy");
        std::fs::create_dir(&sibling).unwrap();
        assert!(!existing_path_contains(&root, &sibling).unwrap());
        assert!(
            prospective_path_relative(&root, &root.join("pending/deep/project"))
                .unwrap()
                .is_some()
        );
        assert!(
            prospective_path_relative(&root, &sibling.join("pending/project"))
                .unwrap()
                .is_none()
        );
        assert!(path_relative(&root, &root.join("../outside")).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn windows_verbatim_drive_unc_case_and_similar_prefixes_are_consistent() {
        assert!(path_components_equal(
            Path::new(r"F:\Project\Assets"),
            Path::new(r"\\?\f:\project\ASSETS")
        )
        .unwrap());
        assert_eq!(
            path_relative(
                Path::new(r"F:\Project"),
                Path::new(r"\\?\f:\project\Assets\Long.asset")
            )
            .unwrap(),
            Some(PathBuf::from(r"Assets\Long.asset"))
        );
        assert!(path_relative(
            Path::new(r"F:\Project"),
            Path::new(r"\\?\F:\Project-copy\asset")
        )
        .unwrap()
        .is_none());
        assert!(
            path_relative(Path::new(r"F:\Project"), Path::new(r"\\?\G:\Project\asset"))
                .unwrap()
                .is_none()
        );
        assert!(path_components_equal(
            Path::new(r"\\server\share\Project"),
            Path::new(r"\\?\UNC\SERVER\SHARE\project")
        )
        .unwrap());
        assert!(path_relative(
            Path::new(r"\\server\share\Project"),
            Path::new(r"\\?\UNC\server\share-other\Project")
        )
        .unwrap()
        .is_none());
        assert!(path_relative(
            Path::new(r"F:\Project"),
            Path::new(r"\\?\GLOBALROOT\Device\HarddiskVolume1\Project")
        )
        .is_err());
    }
    #[test]
    fn an_existing_alias_to_outside_does_not_authorize_future_children() {
        let base = base();
        let root = base.join("root");
        let outside = base.join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let link = root.join("alias");
        #[cfg(windows)]
        let result = std::os::windows::fs::symlink_dir(&outside, &link);
        #[cfg(unix)]
        let result = std::os::unix::fs::symlink(&outside, &link);
        if let Err(error) = result {
            panic!("Symlink fixture cannot be created: {error}");
        }
        assert!(!existing_path_contains(&root, &link).unwrap());
        assert!(
            prospective_path_relative(&root, &link.join("missing/deep/file"))
                .unwrap()
                .is_none()
        );
        assert!(!path_components_equal(&std::fs::canonicalize(&link).unwrap(), &link).unwrap());
    }
}
