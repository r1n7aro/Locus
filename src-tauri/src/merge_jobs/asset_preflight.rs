//! Resolve only the files the merge will actually write into Editor asset paths.
//! Metadata changes guard their owning asset, and moved/deleted paths remain in
//! the set even when there is no longer a physical destination file.
use std::collections::BTreeSet;
use std::path::{Component, Path};

pub(super) fn project_asset_paths<'a>(
    repository_root: &Path,
    project_root: &Path,
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, String> {
    let mut assets = BTreeSet::new();
    for path in paths {
        if path.is_empty()
            || path.contains('\\')
            || path.contains(':')
            || Path::new(path)
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(format!("Invalid merge apply path: {path}"));
        }
        let absolute = repository_root.join(path);
        let Ok(relative) = absolute.strip_prefix(project_root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if !relative.starts_with("Assets/") && !relative.starts_with("Packages/") {
            continue;
        }
        let owner = relative.strip_suffix(".meta").unwrap_or(&relative);
        assets.insert(owner.to_string());
    }
    Ok(assets.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_guards_owner_once_and_excludes_unrelated_repository_files() {
        let paths = project_asset_paths(
            Path::new("/repo"),
            Path::new("/repo/Unity"),
            [
                "Unity/Assets/Config.asset",
                "Unity/Assets/Config.asset.meta",
                "README.md",
                "Unity/ProjectSettings/TagManager.asset",
                "Unity/Assets/Material.mat",
            ],
        )
        .unwrap();
        assert_eq!(paths, ["Assets/Config.asset", "Assets/Material.mat"]);
    }

    #[test]
    fn move_and_delete_guard_both_existing_and_not_yet_created_paths() {
        let paths = project_asset_paths(
            Path::new("/repo"),
            Path::new("/repo"),
            [
                "Assets/Old.prefab",
                "Assets/New.prefab",
                "Assets/Old.prefab.meta",
                "Packages/local/Config.asset",
            ],
        )
        .unwrap();
        assert_eq!(
            paths,
            [
                "Assets/New.prefab",
                "Assets/Old.prefab",
                "Packages/local/Config.asset"
            ]
        );
    }

    #[test]
    fn relative_traversal_is_rejected_instead_of_silently_skipping_guard() {
        assert!(project_asset_paths(
            Path::new("/repo"),
            Path::new("/repo"),
            ["Assets/../Elsewhere.asset"]
        )
        .is_err());
        assert!(project_asset_paths(
            Path::new("/repo"),
            Path::new("/repo"),
            ["/outside/Assets/File.asset"]
        )
        .is_err());
    }
}
