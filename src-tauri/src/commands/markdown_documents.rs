use crate::{
    error::AppError,
    markdown_attachments as attachments,
    workspace_service::{ProjectRegistry, WorkspaceRef},
};
use base64::Engine;
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, Manager, State};

fn error(value: impl ToString) -> AppError {
    AppError::new("markdown.document", value.to_string())
}

fn document_path(root: &Path, content_path: &str) -> Result<PathBuf, AppError> {
    if content_path.trim().is_empty() {
        return Err(error("A document path is required"));
    }
    let path = attachments::normalize(&root.join(content_path));
    let parent = dunce::canonicalize(
        path.parent()
            .ok_or_else(|| error("Invalid document path"))?,
    )
    .map_err(error)?;
    if !crate::knowledge_source_registry::path_is_within(&parent, root) {
        return Err(error("Document is outside the workspace"));
    }
    Ok(parent.join(
        path.file_name()
            .ok_or_else(|| error("Invalid document path"))?,
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedMarkdownImage {
    path: String,
    relative_path: String,
}

/// Raised while the project has neither a default image folder nor a rule
/// for the document's folder; the editor then asks the user to choose one.
const IMAGE_FOLDER_UNSET: &str = "markdown.image_directory_unset";

/// Workspace-relative path of `document` with `/` separators.
fn workspace_relative(root: &Path, document: &Path) -> String {
    document
        .strip_prefix(root)
        .unwrap_or(document)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Configured image folder for `document`, or `None` when the project has
/// not chosen one yet. Values are normalized again because hand-edited
/// project configs skip the settings validation.
fn configured_image_folder(root: &Path, document: &Path) -> Result<Option<String>, AppError> {
    use crate::settings::definitions::{
        PROJECT_MARKDOWN_IMAGES_DIRECTORY, PROJECT_MARKDOWN_IMAGES_RULES,
    };
    let root_text = root.to_string_lossy();
    let default_folder = attachments::normalize_workspace_folder(&crate::settings::project::get(
        &root_text,
        PROJECT_MARKDOWN_IMAGES_DIRECTORY,
    ))
    .map_err(|reason| {
        AppError::new(
            "markdown.image_directory_invalid",
            format!("{}: {reason}", PROJECT_MARKDOWN_IMAGES_DIRECTORY.name()),
        )
    })?;
    let rules = crate::settings::project::get(&root_text, PROJECT_MARKDOWN_IMAGES_RULES);
    let rules =
        attachments::normalize_image_rules(rules.as_object().unwrap_or(&Default::default()))
            .map_err(|reason| {
                AppError::new(
                    "markdown.image_directory_invalid",
                    format!("{}: {reason}", PROJECT_MARKDOWN_IMAGES_RULES.name()),
                )
            })?;
    Ok(attachments::image_folder_for(
        &workspace_relative(root, document),
        &default_folder,
        &rules,
    ))
}

fn image_folder(root: &Path, document: &Path) -> Result<String, AppError> {
    configured_image_folder(root, document)?.ok_or_else(|| {
        AppError::new(
            IMAGE_FOLDER_UNSET,
            "Choose a folder for pasted images in this project first.",
        )
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownImageTarget {
    /// Workspace root, for converting picked folders to relative paths.
    root: String,
    /// Workspace-relative folder of the document (`""` at the root).
    document_folder: String,
    /// Folder that receives pasted images; `None` until one is chosen.
    folder: Option<String>,
}

/// Where images pasted into `content_path` are stored. Read by the prompt
/// that asks for a folder after an import failed with `IMAGE_FOLDER_UNSET`:
/// the root converts picked folders, the document folder offers a rule.
#[tauri::command]
pub async fn markdown_image_target(
    workspace_ref: WorkspaceRef,
    content_path: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<MarkdownImageTarget, AppError> {
    let scope = super::knowledge::resolve_knowledge_workspace_scope(
        registry.inner().as_ref(),
        &workspace_ref,
    )?;
    let root = dunce::canonicalize(scope.runtime().root()).map_err(error)?;
    let document = document_path(&root, &content_path)?;
    let relative = workspace_relative(&root, &document);
    Ok(MarkdownImageTarget {
        root: root.to_string_lossy().replace('\\', "/"),
        document_folder: relative
            .rsplit_once('/')
            .map_or("", |(parent, _)| parent)
            .to_string(),
        folder: configured_image_folder(&root, &document)?,
    })
}

#[tauri::command]
pub async fn markdown_relative_path(
    workspace_ref: WorkspaceRef,
    content_path: String,
    target_path: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let scope = super::knowledge::resolve_knowledge_workspace_scope(
        registry.inner().as_ref(),
        &workspace_ref,
    )?;
    let root = dunce::canonicalize(scope.runtime().root()).map_err(error)?;
    let document = document_path(&root, &content_path)?;
    Ok(attachments::relative_url(
        &document,
        &root.join(target_path),
    ))
}

#[tauri::command]
pub async fn markdown_import_image(
    workspace_ref: WorkspaceRef,
    content_path: String,
    name: String,
    data: Option<String>,
    source_path: Option<String>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ImportedMarkdownImage, AppError> {
    let scope = super::knowledge::resolve_knowledge_workspace_scope(
        registry.inner().as_ref(),
        &workspace_ref,
    )?;
    let root = dunce::canonicalize(scope.runtime().root()).map_err(error)?;
    let document = document_path(&root, &content_path)?;
    let folder = image_folder(&root, &document)?;
    let bytes = if let Some(data) = data {
        if data.len() > 28 * 1024 * 1024 {
            return Err(error("Image exceeds 20 MiB"));
        }
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(error)?
    } else if let Some(path) = source_path {
        if std::fs::metadata(&path).map_err(error)?.len() > 20 * 1024 * 1024 {
            return Err(error("Image exceeds 20 MiB"));
        }
        std::fs::read(path).map_err(error)?
    } else {
        return Err(error("Image data is required"));
    };
    if bytes.is_empty() || bytes.len() > 20 * 1024 * 1024 {
        return Err(error("Image exceeds 20 MiB or is empty"));
    }
    let extension = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else if name.to_lowercase().ends_with(".svg")
        && std::str::from_utf8(&bytes).is_ok_and(|s| s.contains("<svg"))
    {
        "svg"
    } else {
        return Err(error("Unsupported image format"));
    };
    let directory = root.join(&folder);
    // Reject symlinked attachment directories before creating or writing files.
    let mut ancestor = root.clone();
    for part in Path::new(&folder).components() {
        ancestor.push(part);
        if std::fs::symlink_metadata(&ancestor).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(error("Attachment directory must be inside the workspace"));
        }
    }
    std::fs::create_dir_all(&directory).map_err(error)?;
    if !dunce::canonicalize(&directory)
        .is_ok_and(|directory| crate::knowledge_source_registry::path_is_within(&directory, &root))
    {
        return Err(error("Attachment directory must be inside the workspace"));
    }
    let path = directory.join(format!("image-{}.{}", uuid::Uuid::new_v4(), extension));
    crate::config::atomic_write_config(&path, &bytes).map_err(error)?;
    Ok(ImportedMarkdownImage {
        path: path.to_string_lossy().replace('\\', "/"),
        relative_path: attachments::relative_url(&document, &path),
    })
}

#[tauri::command]
pub async fn markdown_rewrite_image_links(
    workspace_ref: WorkspaceRef,
    content_path: String,
    text: String,
    old_path: String,
    new_path: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let scope = super::knowledge::resolve_knowledge_workspace_scope(
        registry.inner().as_ref(),
        &workspace_ref,
    )?;
    let root = dunce::canonicalize(scope.runtime().root()).map_err(error)?;
    let document = document_path(&root, &content_path)?;
    Ok(attachments::rewrite_image_links(
        &text,
        &root,
        &document,
        Path::new(&old_path),
        Path::new(&new_path),
    ))
}

use crate::document_recovery::{
    DocumentRecoveryStore, RecoveryRead, RecoveryUpdate, RecoveryUpdateResult,
};

async fn with_recovery<T: Send + 'static>(
    store: Arc<DocumentRecoveryStore>,
    work: impl FnOnce(&DocumentRecoveryStore) -> Result<T, String> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(move || work(&store))
        .await
        .map_err(error)?
        .map_err(error)
}

#[tauri::command]
pub async fn markdown_recovery_read(
    key: String,
    include_history: Option<bool>,
    store: State<'_, Arc<DocumentRecoveryStore>>,
) -> Result<RecoveryRead, AppError> {
    let include_history = include_history.unwrap_or(false);
    with_recovery(store.inner().clone(), move |store| {
        store.read(&key, include_history)
    })
    .await
}

#[tauri::command]
pub async fn markdown_recovery_update(
    request: RecoveryUpdate,
    store: State<'_, Arc<DocumentRecoveryStore>>,
) -> Result<RecoveryUpdateResult, AppError> {
    with_recovery(store.inner().clone(), move |store| store.update(request)).await
}

pub(crate) fn recovery_image_updates(
    app: &AppHandle,
    root: &Path,
    checkout: Option<&str>,
    old: &Path,
    new: &Path,
) -> Result<Vec<attachments::ReferenceUpdate>, AppError> {
    let Some(store) = app.try_state::<Arc<DocumentRecoveryStore>>() else {
        return Ok(Vec::new());
    };
    let rewrites = store
        .image_rewrites(checkout, |content_path, text| {
            let document = root.join(content_path);
            attachments::rewrite_image_links(text, root, &document, old, new)
        })
        .map_err(error)?;
    Ok(rewrites
        .into_iter()
        .map(|rewrite| attachments::ReferenceUpdate {
            path: rewrite.path,
            before: rewrite.before,
            after: rewrite.after,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_import_is_scoped_to_workspace() {
        let root = tempfile::tempdir().unwrap();
        let canonical = dunce::canonicalize(root.path()).unwrap();
        std::fs::create_dir(root.path().join("Docs")).unwrap();
        assert!(document_path(&canonical, "Docs/readme.md").is_ok());
        assert!(document_path(&canonical, "../outside.md").is_err());
    }

    #[test]
    fn image_folder_comes_from_project_rules_then_the_default() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        std::fs::create_dir_all(root.join("docs/design")).unwrap();
        std::fs::create_dir_all(root.join("Locus")).unwrap();
        let design = document_path(&root, "docs/design/a.md").unwrap();
        let readme = document_path(&root, "README.md").unwrap();
        let config = root.join("Locus/config.json");

        // Nothing chosen yet: the editor asks.
        assert_eq!(configured_image_folder(&root, &design).unwrap(), None);
        assert_eq!(
            image_folder(&root, &design).unwrap_err().code,
            IMAGE_FOLDER_UNSET
        );

        // Hand-edited values are normalized before matching.
        std::fs::write(
            &config,
            r#"{ "markdown_image_rules": { "docs/design/": "./docs/design/images/" } }"#,
        )
        .unwrap();
        assert_eq!(image_folder(&root, &design).unwrap(), "docs/design/images");
        assert_eq!(configured_image_folder(&root, &readme).unwrap(), None);

        std::fs::write(
            &config,
            r#"{ "markdown_image_directory": "assets", "markdown_image_rules": { "docs/design": "docs/design/images" } }"#,
        )
        .unwrap();
        assert_eq!(image_folder(&root, &design).unwrap(), "docs/design/images");
        assert_eq!(image_folder(&root, &readme).unwrap(), "assets");

        // A hand-edited folder outside the workspace is reported, not used.
        std::fs::write(&config, r#"{ "markdown_image_directory": "../outside" }"#).unwrap();
        assert_eq!(
            image_folder(&root, &readme).unwrap_err().code,
            "markdown.image_directory_invalid"
        );
    }
}
