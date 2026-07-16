//! Additional working directories attached to a workspace.
//!
//! Some projects keep related content outside the Unity project root (an art
//! source directory, a design-doc folder, ...). Users can attach such folders
//! to a workspace together with a comment describing what the folder is for;
//! the full path plus comment is injected into the agent env context.
//!
//! The attachment list is stored per-workspace in
//! `Library/Locus/extra_workdirs.json`. The Unity `Library` folder is
//! machine-local (never under version control), so attachments hold absolute
//! local paths and never sync with the project.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const EXTRA_WORKDIRS_FILE: &str = "extra_workdirs.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtraWorkdirEntry {
    pub path: String,
    #[serde(default)]
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtraWorkdirStatus {
    pub path: String,
    pub comment: String,
    pub exists: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtraWorkdirsConfig {
    #[serde(default)]
    entries: Vec<ExtraWorkdirEntry>,
}

pub fn config_path(workspace_dir: &str) -> PathBuf {
    crate::knowledge_index::library_dir_for_working_dir(workspace_dir).join(EXTRA_WORKDIRS_FILE)
}

pub fn load_entries(workspace_dir: &str) -> Vec<ExtraWorkdirEntry> {
    if workspace_dir.trim().is_empty() {
        return Vec::new();
    }
    let Ok(data) = std::fs::read_to_string(config_path(workspace_dir)) else {
        return Vec::new();
    };
    serde_json::from_str::<ExtraWorkdirsConfig>(&data)
        .map(|config| config.entries)
        .unwrap_or_default()
}

pub fn save_entries(workspace_dir: &str, entries: &[ExtraWorkdirEntry]) -> Result<(), String> {
    let library_dir = crate::knowledge_index::library_dir_for_working_dir(workspace_dir);
    std::fs::create_dir_all(&library_dir)
        .map_err(|e| format!("Failed to create extra workdirs config dir: {}", e))?;
    let config = ExtraWorkdirsConfig {
        entries: entries.to_vec(),
    };
    let data = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize extra workdirs config: {}", e))?;
    std::fs::write(config_path(workspace_dir), data)
        .map_err(|e| format!("Failed to write extra workdirs config: {}", e))
}

/// Path key for dedupe/containment checks: unified separators, no trailing
/// separator. Windows paths compare case-insensitively.
fn normalized_key(path: &str) -> String {
    let unified = path.trim().replace('\\', "/");
    let trimmed = unified.trim_end_matches('/');
    if cfg!(windows) {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

/// Normalizes user-supplied entries: trims paths and comments, drops empty
/// paths, duplicates, and paths at or inside the workspace root (those are
/// already part of the working directory).
pub fn normalize_entries(
    workspace_dir: &str,
    entries: Vec<ExtraWorkdirEntry>,
) -> Vec<ExtraWorkdirEntry> {
    let workspace_key = normalized_key(workspace_dir);
    let mut seen: HashSet<String> = HashSet::new();
    let mut result = Vec::new();
    for entry in entries {
        let path = entry.path.trim();
        if path.is_empty() {
            continue;
        }
        let key = normalized_key(path);
        if key.is_empty() || key == workspace_key {
            continue;
        }
        if !workspace_key.is_empty() && key.starts_with(&format!("{}/", workspace_key)) {
            continue;
        }
        if !seen.insert(key) {
            continue;
        }
        result.push(ExtraWorkdirEntry {
            path: path.to_string(),
            comment: entry.comment.trim().to_string(),
        });
    }
    result
}

pub fn entry_statuses(entries: &[ExtraWorkdirEntry]) -> Vec<ExtraWorkdirStatus> {
    entries
        .iter()
        .map(|entry| ExtraWorkdirStatus {
            path: entry.path.clone(),
            comment: entry.comment.clone(),
            exists: Path::new(&entry.path).is_dir(),
        })
        .collect()
}

pub fn load_statuses(workspace_dir: &str) -> Vec<ExtraWorkdirStatus> {
    entry_statuses(&load_entries(workspace_dir))
}

/// Env-prompt block describing the attached directories. Directories that do
/// not exist right now are skipped — the UI surfaces those to the user; the
/// agent only sees paths it can actually access.
pub fn build_env_prompt_block(workspace_dir: &str) -> Option<String> {
    let entries = load_entries(workspace_dir);
    let lines: Vec<String> = entries
        .iter()
        .filter(|entry| Path::new(&entry.path).is_dir())
        .map(|entry| {
            if entry.comment.trim().is_empty() {
                format!("- {}", entry.path)
            } else {
                format!("- {} — {}", entry.path, entry.comment.trim())
            }
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "## Additional Working Directories\nThe user attached these directories to this workspace as additional working directories (each entry is the full path, followed by the user's note on what the folder is for). They live outside the main working directory: treat them as part of the project scope and access them with absolute paths.\n{}",
        lines.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, comment: &str) -> ExtraWorkdirEntry {
        ExtraWorkdirEntry {
            path: path.to_string(),
            comment: comment.to_string(),
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().to_string_lossy().to_string();
        let entries = vec![entry("D:/Art/Sources", "美术资产目录"), entry("D:/Docs", "")];

        save_entries(&workspace, &entries).unwrap();
        assert_eq!(load_entries(&workspace), entries);
        assert!(config_path(&workspace).starts_with(
            Path::new(&workspace).join("Library").join("Locus")
        ));
    }

    #[test]
    fn load_missing_or_invalid_config_is_empty() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().to_string_lossy().to_string();
        assert!(load_entries(&workspace).is_empty());
        assert!(load_entries("").is_empty());

        let library_dir = crate::knowledge_index::library_dir_for_working_dir(&workspace);
        std::fs::create_dir_all(&library_dir).unwrap();
        std::fs::write(config_path(&workspace), "not json").unwrap();
        assert!(load_entries(&workspace).is_empty());
    }

    #[test]
    fn normalize_drops_empty_duplicate_and_workspace_paths() {
        let workspace = if cfg!(windows) { "C:\\Proj" } else { "/proj" };
        let inside = if cfg!(windows) { "C:\\Proj\\Assets" } else { "/proj/Assets" };
        let duplicate = if cfg!(windows) { "D:/Art/" } else { "/art/" };
        let duplicate_alt = if cfg!(windows) { "D:\\ART" } else { "/art" };
        let normalized = normalize_entries(
            workspace,
            vec![
                entry("  ", "blank"),
                entry(workspace, "self"),
                entry(inside, "inside"),
                entry(duplicate, " first "),
                entry(duplicate_alt, "second"),
                entry("E:/Docs", " note "),
            ],
        );
        assert_eq!(
            normalized,
            vec![
                entry(duplicate.trim(), "first"),
                entry("E:/Docs", "note"),
            ]
        );
    }

    #[test]
    fn env_prompt_block_lists_existing_dirs_with_comments() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("proj");
        let art_dir = temp.path().join("art");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&art_dir).unwrap();
        let workspace = workspace.to_string_lossy().to_string();
        let art = art_dir.to_string_lossy().to_string();
        let missing = temp.path().join("missing").to_string_lossy().to_string();

        save_entries(
            &workspace,
            &[entry(&art, "美术资产目录"), entry(&missing, "gone")],
        )
        .unwrap();

        let block = build_env_prompt_block(&workspace).unwrap();
        assert!(block.starts_with("## Additional Working Directories"));
        assert!(block.contains(&format!("- {} — 美术资产目录", art)));
        assert!(!block.contains(&missing));

        // Only missing dirs configured -> no block at all.
        save_entries(&workspace, &[entry(&missing, "gone")]).unwrap();
        assert_eq!(build_env_prompt_block(&workspace), None);
    }

    #[test]
    fn statuses_report_existence() {
        let temp = tempfile::tempdir().unwrap();
        let existing = temp.path().to_string_lossy().to_string();
        let missing = temp.path().join("missing").to_string_lossy().to_string();
        let statuses = entry_statuses(&[entry(&existing, "a"), entry(&missing, "b")]);
        assert!(statuses[0].exists);
        assert!(!statuses[1].exists);
    }
}
