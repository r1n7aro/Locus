//! Select candidate assets before adding the isolated Editor harness. Ordinary
//! asset integration validates its explicit scope and runtime dependencies;
//! code/import configuration changes require the broader candidate asset set.
use std::collections::BTreeSet;
use std::path::Path;

/// Both validation placements execute the same traversal. The applied Editor
/// reports activity through its bridge; the isolated harness only uses Unity.
pub(super) fn script(paths: &[String], progress: &str) -> String {
    include_str!("unity_validation.cs.txt")
        .replace("__LOCUS_PROGRESS__", progress)
        .replace(
            "__LOCUS_PATHS__",
            &paths
                .iter()
                .map(|path| serde_json::to_string(path).unwrap())
                .collect::<Vec<_>>()
                .join(","),
        )
}

fn global_import_change(path: &str) -> bool {
    let asset = path.strip_suffix(".meta").unwrap_or(path);
    asset.ends_with(".cs")
        || asset.ends_with(".asmdef")
        || asset.ends_with(".asmref")
        || asset.ends_with(".dll")
        || asset.ends_with(".rsp")
        || asset.starts_with("Packages/")
        || asset.starts_with("ProjectSettings/")
}

pub(super) fn paths(
    repo: &Path,
    project: &Path,
    requested: &[String],
) -> Result<(Vec<String>, &'static str), String> {
    let repo = std::fs::canonicalize(repo)
        .map_err(|e| format!("Could not resolve validation repository {}: {e}", repo.display()))?;
    let project = std::fs::canonicalize(project)
        .map_err(|e| format!("Could not resolve validation project {}: {e}", project.display()))?;
    let mut selected = BTreeSet::new();
    let mut global = false;
    for path in requested {
        let full = repo.join(crate::workspace_service::worktrees::safe_relative(path)?);
        let relative = crate::workspace_service::worktrees::path_relative(&project, &full)?
            .ok_or("Validation scope includes another project")?
            .to_string_lossy()
            .replace('\\', "/");
        global |= global_import_change(&relative);
        let asset = relative.strip_suffix(".meta").unwrap_or(&relative);
        if asset.starts_with("Assets/") && project.join(asset).is_file() {
            selected.insert(asset.to_string());
        }
    }
    if global {
        for item in walkdir::WalkDir::new(project.join("Assets")).follow_links(false) {
            let item = item.map_err(|e| e.to_string())?;
            if item.file_type().is_symlink() {
                return Err("Candidate validation does not traverse asset symbolic links".into());
            }
            if item.file_type().is_file() {
                let path = crate::workspace_service::worktrees::path_relative(&project, item.path())?
                    .ok_or("Validation asset escapes its project")?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !path.ends_with(".meta") {
                    selected.insert(path);
                }
            }
        }
    }
    Ok((
        selected.into_iter().collect(),
        if global {
            "all_assets_for_schema_or_import_settings"
        } else {
            "selected_assets_and_unity_dependencies"
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn long_nested_project_scope_accepts_mixed_path_prefixes_and_deleted_assets() {
        let temp = tempfile::tempdir().unwrap();
        let relative_project = format!("{}/{}/{}", "a".repeat(90), "b".repeat(90), "c".repeat(90));
        let project = temp.path().join(&relative_project);
        std::fs::create_dir_all(project.join("Assets")).unwrap();
        std::fs::write(project.join("Assets/Chosen.mat"), b"fixture").unwrap();
        assert!(project.to_string_lossy().len() > 260);
        let requested = vec![
            format!("{relative_project}/Assets/Chosen.mat"),
            format!("{relative_project}/Assets/Deleted.asset"),
        ];
        for project in [project.clone(), std::fs::canonicalize(&project).unwrap()] {
            let (selected, mode) = paths(temp.path(), &project, &requested).unwrap();
            assert_eq!(selected, vec!["Assets/Chosen.mat"]);
            assert_eq!(mode, "selected_assets_and_unity_dependencies");
        }
    }

    #[test]
    fn material_scope_avoids_unrelated_scenes_but_script_metadata_expands() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("Project");
        std::fs::create_dir_all(project.join("Assets")).unwrap();
        for file in ["Chosen.mat", "Unrelated.unity", "Type.cs"] {
            std::fs::write(project.join("Assets").join(file), b"fixture").unwrap();
        }
        let (scope, mode) = paths(
            temp.path(),
            &project,
            &["Project/Assets/Chosen.mat.meta".into()],
        )
        .unwrap();
        assert_eq!(scope, vec!["Assets/Chosen.mat"]);
        assert_eq!(mode, "selected_assets_and_unity_dependencies");
        let (scope, mode) = paths(
            temp.path(),
            &project,
            &["Project/Assets/Type.cs.meta".into()],
        )
        .unwrap();
        assert!(scope.contains(&"Assets/Unrelated.unity".into()));
        assert_eq!(mode, "all_assets_for_schema_or_import_settings");
        assert!(paths(temp.path(), &project, &["Sibling/Assets/Other.mat".into()]).is_err());
    }

    #[test]
    fn existing_dirty_code_and_bridge_do_not_expand_selected_asset_scope() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("Project");
        std::fs::create_dir_all(project.join("Assets")).unwrap();
        std::fs::create_dir_all(project.join("Packages/com.farlocus.locus/Editor")).unwrap();
        for path in [
            "Assets/Graph.asset",
            "Assets/Parent.prefab",
            "Assets/Unrelated.unity",
            "Assets/DirtySchema.cs",
            "Packages/com.farlocus.locus/Editor/DirtyBridge.cs",
        ] {
            std::fs::write(project.join(path), b"dirty or applied source").unwrap();
        }
        let (scope, mode) = paths(
            temp.path(),
            &project,
            &[
                "Project/Assets/Graph.asset".into(),
                "Project/Assets/Graph.asset.meta".into(),
                "Project/Assets/Parent.prefab".into(),
                "Project/Assets/Deleted.asset".into(),
            ],
        )
        .unwrap();
        assert_eq!(scope, vec!["Assets/Graph.asset", "Assets/Parent.prefab"]);
        assert_eq!(mode, "selected_assets_and_unity_dependencies");
        for selected in [
            "Assets/DirtySchema.cs",
            "Packages/com.farlocus.locus/Editor/DirtyBridge.cs",
            "ProjectSettings/EditorSettings.asset",
        ] {
            let (expanded, mode) = paths(
                temp.path(),
                &project,
                &[format!("Project/{selected}")],
            )
            .unwrap();
            assert!(expanded.contains(&"Assets/Unrelated.unity".into()));
            assert_eq!(mode, "all_assets_for_schema_or_import_settings");
        }
    }

    #[test]
    fn template_binds_progress_to_the_validation_host_and_escapes_asset_paths() {
        let paths = vec!["Assets/Quoted\"Name.asset".into()];
        let applied = script(&paths, "message => print(message)");
        let isolated = script(&paths, "message => UnityEngine.Debug.Log(message)");
        assert!(applied.contains("message => print(message)"));
        assert!(isolated.contains("message => UnityEngine.Debug.Log(message)"));
        assert!(!isolated.contains("print("));
        for source in [applied, isolated] {
            assert!(source.contains(r#""Assets/Quoted\"Name.asset""#));
            assert!(!source.contains("__LOCUS_"));
        }
    }
}
