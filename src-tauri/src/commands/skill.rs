use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::knowledge_index::KnowledgeIndexState;
use crate::knowledge_store::{self, KnowledgeDocument, KnowledgeType, SkillSurface};
use crate::workspace::Workspace;

use super::knowledge::{
    get_updated_at, load_skill_config, reconcile_and_emit_knowledge_changed, AppKnowledgeDir,
    SkillConfig,
};

// ── Manifest ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub argument_hint: String,
    pub dir_name: String,
    pub source: String,
    pub rel_path: String,
    pub updated_at: i64,
    pub skill_enabled: bool,
    pub skill_surface: SkillSurface,
    pub skill_description: Option<String>,
    pub command_trigger: String,
}

// ── Config key helpers ───────────────────────────────────────

const SKILL_DIR_NAME: &str = "skill";

/// Build the canonical config key for a skill document.
fn config_key(source: &str, dir_name: &str) -> String {
    format!("{}:skill/{}", source, dir_name)
}

pub fn skill_item_id(source: &str, dir_name: &str) -> String {
    format!("skill:{}:{}", source, dir_name)
}

pub fn parse_skill_item_id(item_id: &str) -> Option<(&str, &str)> {
    let rest = item_id.strip_prefix("skill:")?;
    let (source, dir_name) = rest.split_once(':')?;
    if source.is_empty() || dir_name.is_empty() {
        return None;
    }
    Some((source, dir_name))
}

pub fn lookup_skill_config(
    configs: &std::collections::HashMap<String, SkillConfig>,
    source: &str,
    dir_name: &str,
) -> SkillConfig {
    let new_key = config_key(source, dir_name);
    configs.get(&new_key).cloned().unwrap_or_default()
}

// ── Scanning ─────────────────────────────────────────────────

fn find_skill_dir(knowledge_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let canonical = knowledge_dir.join(SKILL_DIR_NAME);
    canonical.is_dir().then_some(canonical)
}

fn scan_skill_dir(
    knowledge_dir: &std::path::Path,
    source: &str,
    configs: &std::collections::HashMap<String, SkillConfig>,
) -> Vec<SkillManifest> {
    let skill_dir = match find_skill_dir(knowledge_dir) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let entries = match std::fs::read_dir(&skill_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut manifests = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };

        let rel_path = format!("{}/{}", SKILL_DIR_NAME, name);
        let Ok(document) =
            knowledge_store::load_document_by_root(knowledge_dir, KnowledgeType::Skill, &name)
        else {
            continue;
        };
        let cfg = (source == "app").then(|| lookup_skill_config(configs, source, stem));
        manifests.push(build_skill_manifest(
            &document,
            stem,
            source,
            &rel_path,
            get_updated_at(&path),
            cfg.as_ref(),
        ));
    }

    manifests
}

fn normalize_command_trigger(value: &str, fallback: &str) -> String {
    let seed = if value.trim().is_empty() {
        fallback.trim()
    } else {
        value.trim()
    };
    let trimmed = seed.trim_start_matches('/').trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{}", trimmed)
    }
}

fn resolve_command_trigger(config: &SkillConfig, fallback: &str) -> String {
    normalize_command_trigger(&config.command_trigger, fallback)
}

fn resolve_document_command_trigger(document: &KnowledgeDocument, fallback: &str) -> String {
    normalize_command_trigger(document.command_trigger.as_deref().unwrap_or(""), fallback)
}

fn build_skill_manifest(
    document: &KnowledgeDocument,
    dir_name: &str,
    source: &str,
    rel_path: &str,
    updated_at: i64,
    override_config: Option<&SkillConfig>,
) -> SkillManifest {
    let skill_enabled = override_config
        .map(|config| config.enabled)
        .unwrap_or_else(|| document.skill_enabled.unwrap_or(true));
    let skill_surface = override_config
        .map(|config| config.surface)
        .unwrap_or_else(|| document.skill_surface.unwrap_or_default());
    let manifest_description = knowledge_store::active_summary(document)
        .unwrap_or_default()
        .to_string();
    let skill_description = override_config
        .and_then(|config| {
            (!config.description.trim().is_empty()).then(|| config.description.clone())
        })
        .or_else(|| {
            (!manifest_description.trim().is_empty()).then(|| manifest_description.clone())
        });
    let command_trigger = override_config
        .map(|config| resolve_command_trigger(config, &document.title))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| resolve_document_command_trigger(document, &document.title));

    SkillManifest {
        name: document.title.clone(),
        description: manifest_description,
        argument_hint: document.argument_hint.clone().unwrap_or_default(),
        dir_name: dir_name.to_string(),
        source: source.to_string(),
        rel_path: rel_path.to_string(),
        updated_at,
        skill_enabled,
        skill_surface,
        skill_description,
        command_trigger,
    }
}

// ── Tauri commands ───────────────────────────────────────────

#[tauri::command]
pub async fn list_skills(
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
) -> Result<Vec<SkillManifest>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    Ok(list_skills_sync(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
    ))
}

pub fn list_skills_sync(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
) -> Vec<SkillManifest> {
    let configs = load_skill_config(working_dir);
    let mut manifests = Vec::new();

    if let Some(app_dir) = app_knowledge_dir {
        manifests.extend(scan_skill_dir(app_dir, "app", &configs));
    }

    let project_dir = std::path::Path::new(working_dir)
        .join("Locus")
        .join("knowledge");
    if project_dir.is_dir() {
        let project_skills = scan_skill_dir(&project_dir, "project", &configs);
        for ps in project_skills {
            manifests.retain(|m| m.dir_name != ps.dir_name);
            manifests.push(ps);
        }
    }

    manifests.sort_by(|a, b| a.name.cmp(&b.name));
    manifests
}

pub fn resolve_skill_manifest_path_sync(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    dir_name: &str,
    source: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    if dir_name.contains("..") || dir_name.contains('/') || dir_name.contains('\\') {
        return Err("Invalid skill name".to_string());
    }

    let src = source.unwrap_or("project");
    let knowledge_dir = if src == "app" {
        app_knowledge_dir
            .cloned()
            .ok_or_else(|| "App knowledge directory not found".to_string())?
    } else {
        std::path::Path::new(working_dir)
            .join("Locus")
            .join("knowledge")
    };

    let file_path = knowledge_dir
        .join(SKILL_DIR_NAME)
        .join(format!("{}.md", dir_name));
    if file_path.is_file() {
        return Ok(file_path);
    }

    Err(format!("Skill not found: {}", dir_name))
}

pub fn read_skill_manifest_sync(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    dir_name: &str,
    source: Option<&str>,
) -> Result<String, String> {
    let path = resolve_skill_manifest_path_sync(working_dir, app_knowledge_dir, dir_name, source)?;
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read skill: {}", e))
}

#[tauri::command]
pub async fn read_skill_manifest(
    dir_name: String,
    source: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
) -> Result<String, AppError> {
    let working_dir = workspace.path.read().await.clone();
    read_skill_manifest_sync(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        &dir_name,
        source.as_deref(),
    )
    .map_err(Into::into)
}

const SKILL_SCAFFOLD_BODY: &str = r#"## When to use

## When NOT to use

## Instructions
"#;

fn is_valid_skill_scaffold_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

#[tauri::command]
pub async fn create_skill_scaffold(
    name: String,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<SkillManifest, AppError> {
    if name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !is_valid_skill_scaffold_name(&name)
    {
        return Err("Invalid skill name: use lowercase-kebab-case"
            .to_string()
            .into());
    }

    let working_dir = workspace.path.read().await.clone();
    let title = name
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let document = knowledge_store::KnowledgeDocument {
        id: format!("kd_{}", uuid::Uuid::new_v4()),
        doc_type: KnowledgeType::Skill,
        path: format!("{}.md", name),
        title,
        scope: knowledge_store::KnowledgeScope::Project,
        inject_mode: knowledge_store::KnowledgeInjectMode::None,
        inherit_inject_mode: true,
        inject_mode_source: Default::default(),
        summary_enabled: true,
        command_enabled: true,
        read_only: false,
        ai_maintained: false,
        inherit_ai_config: true,
        ai_config_source: Default::default(),
        explicit_maintenance_rules: false,
        external_source: None,
        skill_enabled: Some(true),
        skill_surface: Some(SkillSurface::Command),
        command_trigger: Some(format!("/{}", name)),
        argument_hint: None,
        summary: None,
        body: SKILL_SCAFFOLD_BODY.to_string(),
        maintenance_rules: None,
        created_at: 0,
        updated_at: 0,
    };
    let saved = knowledge_store::save_document(&working_dir, document)?;
    reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state.inner().clone(),
        "create_skill_scaffold",
    )
    .await?;
    let manifest_path = std::path::Path::new(&working_dir)
        .join("Locus")
        .join("knowledge")
        .join(SKILL_DIR_NAME)
        .join(format!("{}.md", name));

    Ok(build_skill_manifest(
        &saved,
        &name,
        "project",
        &format!("{}/{}.md", SKILL_DIR_NAME, name),
        get_updated_at(&manifest_path),
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::{is_valid_skill_scaffold_name, list_skills_sync};
    use tempfile::TempDir;

    #[test]
    fn skill_scaffold_name_validation_rejects_non_kebab_case_inputs() {
        assert!(is_valid_skill_scaffold_name("asset-audit"));
        assert!(is_valid_skill_scaffold_name("asset-audit-2"));
        assert!(!is_valid_skill_scaffold_name("AssetAudit"));
        assert!(!is_valid_skill_scaffold_name("asset_audit"));
        assert!(!is_valid_skill_scaffold_name("asset--audit"));
        assert!(!is_valid_skill_scaffold_name("-asset-audit"));
        assert!(!is_valid_skill_scaffold_name("asset-audit-"));
    }

    #[test]
    fn list_skills_sync_reads_project_root_skill() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let skill_dir = temp.path().join("Locus").join("knowledge").join("skill");
        std::fs::create_dir_all(&skill_dir).unwrap();

        let raw = r#"---
id: kd_skill_create_skill
type: skill
path: create-skill.md
title: Create Skill
scope: project
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /create-skill
argumentHint: <skill-name>
createdAt: 1
updatedAt: 1
---

# Create Skill

## Summary
Create a project skill.

## Content
## When to use

- Reuse a workflow.
"#;
        std::fs::write(skill_dir.join("create-skill.md"), raw).unwrap();

        let skills = list_skills_sync(&working_dir, None);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].dir_name, "create-skill");
        assert_eq!(skills[0].source, "project");
        assert_eq!(skills[0].command_trigger, "/create-skill");
    }
}
