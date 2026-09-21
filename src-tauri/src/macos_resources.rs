//! Bundle resources live alongside Contents/MacOS, never inside it.
use std::path::{Path, PathBuf};

fn resource_root_for_executable(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    if macos.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    Some(contents.join("Resources"))
}

pub(crate) fn resource_root() -> Option<PathBuf> {
    resource_root_for_executable(&std::env::current_exe().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_bundle_resources_without_a_working_directory_dependency() {
        assert_eq!(
            resource_root_for_executable(Path::new("/Applications/Locus.app/Contents/MacOS/locus")),
            Some(PathBuf::from("/Applications/Locus.app/Contents/Resources"))
        );
        assert_eq!(
            resource_root_for_executable(Path::new("/repo/target/debug/locus")),
            None
        );
    }
}
