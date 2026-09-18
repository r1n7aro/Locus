//! A single-component View stores author metadata in an optional leading Vue
//! custom block. The runtime manifest is derived, never written as a sidecar.
use super::{
    normalize_package_rel_path, normalize_view_id, validate_view_manifest, ViewCapabilities,
    ViewManifest, ViewRequirements, ViewScriptManifest, VIEW_API_VERSION, VIEW_SCHEMA,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unity: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ViewScriptManifest>,
}

pub(super) fn file_name(value: &str) -> Result<String, String> {
    let value = value.trim();
    let normalized = normalize_package_rel_path(value)?;
    if normalized != value || normalized.contains('/') || !normalized.ends_with(".vue") {
        return Err("View fileName must be a single lowercase-kebab-case.vue file name.".into());
    }
    normalize_view_id(normalized.trim_end_matches(".vue"))?;
    Ok(normalized)
}

pub(super) fn source_path(root: &Path) -> Result<Option<PathBuf>, String> {
    if root.join("view.json").is_file() || !root.is_dir() {
        return Ok(None);
    }
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_file()
            && entry.path().extension().and_then(|s| s.to_str()) == Some("vue")
        {
            candidates.push(entry.path());
        }
    }
    if candidates.len() > 1 {
        return Err(format!(
            "Single-file View must contain one root Vue component: {}",
            root.display()
        ));
    }
    Ok(candidates.pop())
}

// Only a leading custom block is metadata. A <view> string inside a script,
// template or comment must never be mistaken for configuration.
fn header(source: &str) -> Result<Option<(usize, usize, &str)>, String> {
    let body = source.trim_start_matches('\u{feff}').trim_start();
    let opening = if body.starts_with("<view>") {
        "<view>"
    } else if body.starts_with("<view lang=\"json\">") {
        "<view lang=\"json\">"
    } else {
        return Ok(None);
    };
    let start = source.len() - body.len();
    let end = body
        .find("</view>")
        .ok_or("View metadata is missing </view>.")?;
    Ok(Some((
        start,
        start + end + "</view>".len(),
        &body[opening.len()..end],
    )))
}

pub(super) fn metadata(source: &str) -> Result<Metadata, String> {
    if source.trim().is_empty() {
        return Err("View component source cannot be empty.".into());
    }
    if source.len() > 8 * 1024 * 1024 {
        return Err("View component source exceeds 8 MiB.".into());
    }
    match header(source)? {
        Some((_, _, raw)) => {
            serde_json::from_str(raw).map_err(|e| format!("Invalid <view> metadata: {e}"))
        }
        None => Ok(Metadata::default()),
    }
}

pub(super) fn with_metadata(source: &str, value: &Metadata) -> Result<String, String> {
    if &metadata(source)? == value {
        return Ok(source.to_string());
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| e.to_string())?
        .replace("</view>", "<\\/view>");
    let block = format!("<view>\n{json}\n</view>");
    match header(source)? {
        Some((start, end, _)) => Ok(format!("{}{block}{}", &source[..start], &source[end..])),
        None => Ok(format!("{block}\n\n{source}")),
    }
}

pub(super) fn manifest(file: &str, source: &str) -> Result<ViewManifest, String> {
    let entry = file_name(file)?;
    let id = entry.strip_suffix(".vue").unwrap().to_string();
    let meta = metadata(source)?;
    let unity = meta.unity.unwrap_or(!meta.scripts.is_empty());
    let result = ViewManifest {
        schema: VIEW_SCHEMA.to_string(),
        api_version: VIEW_API_VERSION.to_string(),
        name: meta.name.unwrap_or_else(|| id.clone()),
        id,
        version: "0.1.0".to_string(),
        template: String::new(),
        display_path: meta.display_path,
        icon: meta.icon.or_else(|| Some("View".to_string())),
        entry,
        style: String::new(),
        scripts: meta.scripts,
        capabilities: ViewCapabilities { unity },
        requirements: Some(ViewRequirements {
            unity_connection: unity,
        }),
    };
    validate_view_manifest(&result)?;
    Ok(result)
}

pub(super) fn write_manifest(path: &Path, value: &ViewManifest) -> Result<(), String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let current = manifest(
        path.file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid View file name")?,
        &source,
    )?;
    let mut meta = metadata(&source)?;
    if value.name != current.name {
        meta.name = Some(value.name.clone());
    }
    if value.icon != current.icon {
        meta.icon = value.icon.clone();
    }
    if value.display_path != current.display_path {
        meta.display_path = value.display_path.clone();
    }
    if value.capabilities.unity != current.capabilities.unity {
        meta.unity = Some(value.capabilities.unity);
    }
    if value.scripts != current.scripts {
        meta.scripts = value.scripts.clone();
    }
    super::write_text_file_atomic(path, &with_metadata(&source, &meta)?)
}
