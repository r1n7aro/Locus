use std::path::{Path, PathBuf};

use chrono::Utc;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::unity_docs;

const KNOWLEDGE_ROOT_DIR: &str = "Locus/knowledge";
const DOCUMENT_LOAD_PARALLEL_THRESHOLD: usize = 48;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeType {
    Design,
    Memory,
    Skill,
    Reference,
}

impl KnowledgeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Memory => "memory",
            Self::Skill => "skill",
            Self::Reference => "reference",
        }
    }

    pub fn all() -> [Self; 4] {
        [Self::Design, Self::Memory, Self::Skill, Self::Reference]
    }
}

impl std::fmt::Display for KnowledgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str((*self).as_str())
    }
}

impl Default for KnowledgeType {
    fn default() -> Self {
        Self::Design
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SkillSurface {
    Command,
    Auto,
    Both,
}

impl SkillSurface {
    pub fn allows_auto(self) -> bool {
        matches!(self, Self::Auto | Self::Both)
    }

    pub fn allows_command(self) -> bool {
        matches!(self, Self::Command | Self::Both)
    }
}

impl Default for SkillSurface {
    fn default() -> Self {
        Self::Command
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeInjectMode {
    None,
    Path,
    Excerpt,
    Full,
    Rule,
}

impl KnowledgeInjectMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Path => "path",
            Self::Excerpt => "excerpt",
            Self::Full => "full",
            Self::Rule => "rule",
        }
    }
}

impl std::fmt::Display for KnowledgeInjectMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str((*self).as_str())
    }
}

impl Default for KnowledgeInjectMode {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceProvider {
    LocalFolder,
    Feishu,
    Url,
    Unity,
    Package,
    Custom,
}

impl KnowledgeSourceProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalFolder => "local_folder",
            Self::Feishu => "feishu",
            Self::Url => "url",
            Self::Unity => "unity",
            Self::Package => "package",
            Self::Custom => "custom",
        }
    }
}

impl std::fmt::Display for KnowledgeSourceProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str((*self).as_str())
    }
}

impl Default for KnowledgeSourceProvider {
    fn default() -> Self {
        Self::Custom
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeExternalSource {
    pub provider: KnowledgeSourceProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default)]
    pub sync_enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeStorageSource {
    #[default]
    Project,
    App,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeConfigSourceKind {
    #[default]
    #[serde(rename = "self")]
    SelfValue,
    ParentDirectory,
    TypeDefault,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeConfigSource {
    pub kind: KnowledgeConfigSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDocument {
    pub id: String,
    #[serde(rename = "type")]
    pub doc_type: KnowledgeType,
    pub path: String,
    pub title: String,
    pub inject_mode: KnowledgeInjectMode,
    #[serde(default)]
    pub inherit_inject_mode: bool,
    pub inject_mode_source: KnowledgeConfigSource,
    pub summary_enabled: bool,
    pub command_enabled: bool,
    pub read_only: bool,
    pub ai_maintained: bool,
    #[serde(default, skip_deserializing)]
    pub storage_source: KnowledgeStorageSource,
    #[serde(default)]
    pub inherit_ai_config: bool,
    pub ai_config_source: KnowledgeConfigSource,
    pub explicit_maintenance_rules: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_source: Option<KnowledgeExternalSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_surface: Option<SkillSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_rules: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct KnowledgeFrontmatter {
    pub id: String,
    #[serde(rename = "type")]
    pub doc_type: KnowledgeType,
    #[serde(default)]
    pub path: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inject_mode: Option<KnowledgeInjectMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_inject_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_cache: Option<String>,
    #[serde(default)]
    pub command_enabled: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_maintained: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_ai_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_maintenance_rules: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_rules_cache: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_source: Option<KnowledgeExternalSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_surface: Option<SkillSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeListItem {
    pub id: String,
    #[serde(rename = "type")]
    pub doc_type: KnowledgeType,
    pub path: String,
    pub title: String,
    pub inject_mode: KnowledgeInjectMode,
    pub summary_enabled: bool,
    pub command_enabled: bool,
    pub read_only: bool,
    pub ai_maintained: bool,
    pub explicit_maintenance_rules: bool,
    #[serde(default)]
    pub storage_source: KnowledgeStorageSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_source: Option<KnowledgeExternalSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_surface: Option<SkillSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub has_summary: bool,
    #[serde(default)]
    pub has_body_content: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lexical_search_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_search_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

pub fn list_item_has_injectable_content(item: &KnowledgeListItem) -> bool {
    match item.inject_mode {
        KnowledgeInjectMode::None => false,
        KnowledgeInjectMode::Path => true,
        KnowledgeInjectMode::Excerpt => item.has_summary || item.has_body_content,
        KnowledgeInjectMode::Full => true,
        KnowledgeInjectMode::Rule => true,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeReadResult {
    #[serde(flatten)]
    pub document: KnowledgeDocument,
    pub part: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_metadata: Option<KnowledgeDocumentFileMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDocumentFileMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_commit_author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_commit_at: Option<i64>,
}

fn default_directory_config_version() -> u32 {
    4
}

fn default_true() -> bool {
    true
}

fn default_directory_inject_mode() -> KnowledgeInjectMode {
    KnowledgeInjectMode::Excerpt
}

fn default_enabled_capability_state() -> EffectiveCapabilityState {
    EffectiveCapabilityState {
        enabled: true,
        source: "default".to_string(),
        reason_code: None,
        source_dir: None,
    }
}

fn self_capability_state(enabled: bool) -> EffectiveCapabilityState {
    EffectiveCapabilityState {
        enabled,
        source: "self".to_string(),
        reason_code: None,
        source_dir: None,
    }
}

fn default_folder_index_rule_setting() -> FolderIndexRuleSetting {
    FolderIndexRuleSetting::Inherit
}

fn folder_index_rule_setting_is_inherit(value: &FolderIndexRuleSetting) -> bool {
    *value == FolderIndexRuleSetting::Inherit
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FolderIndexRuleSetting {
    #[default]
    Inherit,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveCapabilityState {
    pub enabled: bool,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirectorySearchAccess {
    pub lexical_enabled: bool,
    pub vector_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDirectoryConfig {
    #[serde(default = "default_directory_config_version")]
    pub version: u32,
    #[serde(default)]
    pub summary: String,
    #[serde(default = "default_directory_inject_mode")]
    pub inject_mode: KnowledgeInjectMode,
    #[serde(default)]
    pub inherit_inject_mode: bool,
    #[serde(default)]
    pub ai_maintained: bool,
    #[serde(default)]
    pub inherit_ai_config: bool,
    #[serde(default)]
    pub explicit_maintenance_rules: bool,
    #[serde(default = "default_folder_index_rule_setting")]
    pub lexical_search: FolderIndexRuleSetting,
    #[serde(default = "default_folder_index_rule_setting")]
    pub vector_search: FolderIndexRuleSetting,
    #[serde(default = "default_true")]
    pub inherit_to_children: bool,
    #[serde(default = "default_true")]
    pub allow_create_documents: bool,
    #[serde(default = "default_true")]
    pub allow_create_directories: bool,
    #[serde(default = "default_true")]
    pub allow_move_documents: bool,
    #[serde(default = "default_true")]
    pub allow_move_directories: bool,
    #[serde(default)]
    pub maintenance_rules: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StoredKnowledgeDirectoryConfig {
    #[serde(default = "default_directory_config_version")]
    pub version: u32,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inject_mode: Option<KnowledgeInjectMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_inject_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_maintained: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_ai_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_maintenance_rules: Option<bool>,
    #[serde(
        default = "default_folder_index_rule_setting",
        skip_serializing_if = "folder_index_rule_setting_is_inherit"
    )]
    pub lexical_search: FolderIndexRuleSetting,
    #[serde(
        default = "default_folder_index_rule_setting",
        skip_serializing_if = "folder_index_rule_setting_is_inherit"
    )]
    pub vector_search: FolderIndexRuleSetting,
    #[serde(default = "default_true")]
    pub inherit_to_children: bool,
    #[serde(default = "default_true")]
    pub allow_create_documents: bool,
    #[serde(default = "default_true")]
    pub allow_create_directories: bool,
    #[serde(default = "default_true")]
    pub allow_move_documents: bool,
    #[serde(default = "default_true")]
    pub allow_move_directories: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_rules: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_sources: Vec<KnowledgeExternalSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDirectoryConfigRecord {
    #[serde(rename = "type")]
    pub doc_type: KnowledgeType,
    pub path: String,
    pub config_path: String,
    pub exists: bool,
    #[serde(default)]
    pub read_only: bool,
    pub updated_at: i64,
    pub inject_mode_source: KnowledgeConfigSource,
    pub ai_config_source: KnowledgeConfigSource,
    pub effective_lexical_search: EffectiveCapabilityState,
    pub effective_vector_search: EffectiveCapabilityState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_sources: Vec<KnowledgeExternalSource>,
    #[serde(flatten)]
    pub config: KnowledgeDirectoryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeExternalDirectoryBinding {
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_sources: Vec<KnowledgeExternalSource>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeTargetKind {
    #[default]
    Document,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDocumentPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<KnowledgeType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inject_mode: Option<KnowledgeInjectMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_inject_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_maintained: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_ai_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_maintenance_rules: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_source: Option<Option<KnowledgeExternalSource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_surface: Option<SkillSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_trigger: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_rules: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDirectoryConfigPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inject_mode: Option<KnowledgeInjectMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_inject_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_maintained: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_ai_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_maintenance_rules: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lexical_search: Option<FolderIndexRuleSetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_search: Option<FolderIndexRuleSetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_to_children: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_create_documents: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_create_directories: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_move_documents: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_move_directories: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_rules: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeReadRequest {
    #[serde(default)]
    pub kind: KnowledgeTargetKind,
    pub path: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<KnowledgeType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeCreateRequest {
    #[serde(default)]
    pub kind: KnowledgeTargetKind,
    pub path: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<KnowledgeType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<KnowledgeDocumentPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEditRequest {
    #[serde(default)]
    pub kind: KnowledgeTargetKind,
    pub path: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<KnowledgeType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<KnowledgeDocumentPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<KnowledgeDirectoryConfigPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeMoveRequest {
    #[serde(default)]
    pub kind: KnowledgeTargetKind,
    pub path: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<KnowledgeType>,
    pub new_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDeleteRequest {
    #[serde(default)]
    pub kind: KnowledgeTargetKind,
    pub path: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<KnowledgeType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeReadResponse {
    pub kind: KnowledgeTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<KnowledgeReadResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<KnowledgeDirectoryConfigRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeMutationResponse {
    pub kind: KnowledgeTargetKind,
    #[serde(rename = "type")]
    pub doc_type: KnowledgeType,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<KnowledgeDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<KnowledgeDirectoryConfigRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSearchHit {
    pub id: String,
    #[serde(rename = "type")]
    pub doc_type: KnowledgeType,
    pub path: String,
    pub title: String,
    #[serde(default)]
    pub storage_source: KnowledgeStorageSource,
    pub inject_mode: KnowledgeInjectMode,
    pub ai_maintained: bool,
    pub score: f32,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_section: Option<KnowledgeSearchMatchSection>,
    pub has_summary: bool,
    pub updated_at: i64,
    #[serde(default)]
    pub match_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum KnowledgeSearchMatchSection {
    Summary,
    MaintenanceRules,
    Body,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeUpdateOp {
    Create,
    Edit,
    UpdateMeta,
    UpdateSummary,
    UpdateBody,
    UpdateRules,
    Delete,
}

impl Default for KnowledgeUpdateOp {
    fn default() -> Self {
        Self::Create
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeUpdateRequest {
    pub op: KnowledgeUpdateOp,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<KnowledgeType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inject_mode: Option<KnowledgeInjectMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_inject_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_maintained: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_ai_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_maintenance_rules: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_source: Option<Option<KnowledgeExternalSource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_surface: Option<SkillSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_trigger: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_rules: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_path: Option<String>,
}

fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

fn provider_is_read_only(provider: KnowledgeSourceProvider) -> bool {
    matches!(
        provider,
        KnowledgeSourceProvider::LocalFolder
            | KnowledgeSourceProvider::Feishu
            | KnowledgeSourceProvider::Package
    )
}

fn derive_read_only(external_source: Option<&KnowledgeExternalSource>) -> bool {
    external_source
        .map(|source| provider_is_read_only(source.provider))
        .unwrap_or(false)
}

fn self_config_source() -> KnowledgeConfigSource {
    KnowledgeConfigSource {
        kind: KnowledgeConfigSourceKind::SelfValue,
        path: None,
    }
}

fn type_default_config_source() -> KnowledgeConfigSource {
    KnowledgeConfigSource {
        kind: KnowledgeConfigSourceKind::TypeDefault,
        path: None,
    }
}

fn parent_directory_config_source(path: &str) -> KnowledgeConfigSource {
    KnowledgeConfigSource {
        kind: KnowledgeConfigSourceKind::ParentDirectory,
        path: Some(path.to_string()),
    }
}

pub fn default_ai_maintained_for_type(doc_type: KnowledgeType) -> bool {
    matches!(doc_type, KnowledgeType::Memory)
}

pub fn default_explicit_maintenance_rules_for_type(doc_type: KnowledgeType) -> bool {
    matches!(doc_type, KnowledgeType::Memory)
}

pub fn default_summary_enabled_for_type(doc_type: KnowledgeType) -> bool {
    matches!(doc_type, KnowledgeType::Reference | KnowledgeType::Skill)
}

pub fn default_document_inject_mode_for_type(doc_type: KnowledgeType) -> KnowledgeInjectMode {
    match doc_type {
        KnowledgeType::Design => KnowledgeInjectMode::Path,
        KnowledgeType::Memory | KnowledgeType::Skill | KnowledgeType::Reference => {
            KnowledgeInjectMode::None
        }
    }
}

fn default_document_inherited_config_for_type(doc_type: KnowledgeType) -> KnowledgeDirectoryConfig {
    let ai_maintained = default_ai_maintained_for_type(doc_type);
    let explicit_maintenance_rules = default_explicit_maintenance_rules_for_type(doc_type);
    KnowledgeDirectoryConfig {
        version: default_directory_config_version(),
        summary: String::new(),
        inject_mode: default_document_inject_mode_for_type(doc_type),
        inherit_inject_mode: true,
        ai_maintained,
        inherit_ai_config: true,
        explicit_maintenance_rules,
        lexical_search: FolderIndexRuleSetting::Inherit,
        vector_search: FolderIndexRuleSetting::Inherit,
        inherit_to_children: true,
        allow_create_documents: true,
        allow_create_directories: true,
        allow_move_documents: true,
        allow_move_directories: true,
        maintenance_rules: if explicit_maintenance_rules || ai_maintained {
            default_maintenance_rules_for_type(doc_type)
                .unwrap_or_default()
                .to_string()
        } else {
            String::new()
        },
    }
}

pub fn default_maintenance_rules_for_type(doc_type: KnowledgeType) -> Option<&'static str> {
    Some(match doc_type {
        KnowledgeType::Design => {
            "- Record confirmed design decisions and constraints\n- Keep open questions current and remove outdated approaches\n- Preserve the existing document structure while updating details"
        }
        KnowledgeType::Memory => {
            "- Keep only durable and reusable project memory\n- Consolidate duplicates or conflicts into the latest conclusion\n- Remove temporary context, one-off tasks, and unsupported guesses"
        }
        KnowledgeType::Skill => {
            "- Keep workflow steps, checks, and outputs stable\n- Update key steps when tools or process changes\n- Remove obsolete commands, repeated notes, and expired limits"
        }
        KnowledgeType::Reference => {
            "- Capture verifiable facts, interfaces, and constraints\n- Prefer durable conclusions and version-specific differences\n- Remove outdated examples, repeated summaries, and unverified details"
        }
    })
}

pub fn default_directory_config_for_type(doc_type: KnowledgeType) -> KnowledgeDirectoryConfig {
    let ai_maintained = default_ai_maintained_for_type(doc_type);
    let explicit_maintenance_rules = default_explicit_maintenance_rules_for_type(doc_type);
    KnowledgeDirectoryConfig {
        version: default_directory_config_version(),
        summary: String::new(),
        inject_mode: default_directory_inject_mode(),
        inherit_inject_mode: true,
        ai_maintained,
        inherit_ai_config: true,
        explicit_maintenance_rules,
        lexical_search: FolderIndexRuleSetting::Inherit,
        vector_search: FolderIndexRuleSetting::Inherit,
        inherit_to_children: true,
        allow_create_documents: true,
        allow_create_directories: true,
        allow_move_documents: true,
        allow_move_directories: true,
        maintenance_rules: if explicit_maintenance_rules || ai_maintained {
            default_maintenance_rules_for_type(doc_type)
                .unwrap_or_default()
                .to_string()
        } else {
            String::new()
        },
    }
}

const MEMORY_BUILTIN_SEED_VERSION: u32 = 11;
const KNOWLEDGE_DIRECTORY_CONFIG_SUFFIX: &str = ".locus-meta";
const LEGACY_KNOWLEDGE_DIRECTORY_CONFIG_SUFFIX: &str = ".meta";
const MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH: &str = "unity-project-understanding";
const MEMORY_PROJECT_MISTAKE_NOTE_PATH: &str = "project-mistake-note.md";
const MEMORY_PROJECT_MISTAKE_NOTE_LEGACY_PATH: &str = "project_mistake_note.md";
const MEMORY_USER_PREFERENCE_PATH: &str = "user-preference.md";
const MEMORY_USER_PREFERENCE_LEGACY_PATH: &str = "user_preference.md";
const MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY_V10: &str =
    "维护 Unity 工程理解的结构缓存，沉淀长期有效的目录职责、系统入口、关键资源定位与约束。";
const MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY: &str = "Maintains a structural cache of Unity project understanding, preserving durable directory responsibilities, system entry points, key asset lookup paths, and constraints.";
const MEMORY_PROJECT_MISTAKE_NOTE_RULES_V6: &str = "- 只记录已经验证的问题、返工原因与规避方式\n- 优先维护会重复踩坑的操作、约束、回归点与修复结论\n- 删除已失效问题、无法复现的问题与没有依据的猜测";
const MEMORY_PROJECT_MISTAKE_NOTE_RULES_V9: &str = "- 只记录已经验证的问题、返工原因与规避方式\n- 优先维护会重复踩坑的操作、约束、回归点与修复结论\n- 每条保持简短，聚焦单个教训或约束\n- 总量控制在 20 条以内，定期合并重复项\n- 删除已失效问题、无法复现的问题与没有依据的猜测";
const MEMORY_PROJECT_MISTAKE_NOTE_RULES: &str = "- Record only verified problems, rework causes, and avoidance steps\n- Prioritize recurring pitfalls, constraints, regression points, and confirmed fixes\n- Keep each entry short and focused on one lesson or constraint\n- Keep the list within 20 items and merge duplicates regularly\n- Remove outdated issues, non-reproducible issues, and unsupported guesses";
const MEMORY_USER_PREFERENCE_RULES_V6: &str = "- 只记录跨任务长期稳定的用户偏好\n- 优先维护语言、汇报方式、代码风格、禁忌与明确要求\n- 删除一次性安排、临时口径与没有确认的推断";
const MEMORY_USER_PREFERENCE_RULES_V9: &str = "- 只记录跨任务长期稳定的用户偏好\n- 优先维护语言、汇报方式、代码风格、禁忌与明确要求\n- 每条保持简短，只写稳定偏好或硬约束\n- 总量控制在 20 条以内，合并相近偏好\n- 删除一次性安排、临时口径与没有确认的推断";
const MEMORY_USER_PREFERENCE_RULES: &str = "- Record only long-term user preferences that stay stable across tasks\n- Prioritize language, reporting style, code style, taboos, and explicit requirements\n- Keep each entry short and limited to stable preferences or hard constraints\n- Keep the list within 20 items and merge similar preferences\n- Remove one-off arrangements, temporary phrasing, and unconfirmed inferences";
const MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES_V4: &str = "- 只记录能减少重复探索的 Unity 项目结构认知与定位信息\n- 优先维护目录职责、核心系统入口、关键场景、Prefab、ScriptableObject、程序集与配置映射\n- 记录已经验证的资源关系、运行入口、关键依赖与常用定位路径\n- 删除临时调查过程、一次性任务痕迹、未经验证的猜测与已过期缓存";
const MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES_V9: &str = "- 只记录能减少重复探索的 Unity 项目结构认知与定位信息\n- 只维护来自工程项目本身的工程理解，包括目录职责、系统入口、资源关系、运行入口与配置映射\n- 用户补充的设计目标、玩法意图、产品方向与方案决策写入 Design，不写入这里\n- 优先维护目录职责、核心系统入口、关键场景、Prefab、ScriptableObject、程序集与配置映射\n- 记录已经验证的资源关系、运行入口、关键依赖与常用定位路径\n- 删除临时调查过程、一次性任务痕迹、未经验证的猜测与已过期缓存";
const MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES: &str = "- Record only Unity project structure knowledge and lookup info that reduce repeated exploration\n- Maintain only project-derived engineering understanding, including directory responsibilities, system entry points, asset relationships, runtime entry points, and config mappings\n- Write user-supplied design goals, gameplay intent, product direction, and solution decisions into Design\n- Prioritize directory responsibilities, core system entry points, key scenes, prefabs, ScriptableObjects, assemblies, and config mappings\n- Record verified asset relationships, runtime entry points, key dependencies, and common lookup paths\n- Remove temporary investigation traces, one-off task residue, unverified guesses, and expired cache";

struct MemoryBuiltinSeed {
    id: &'static str,
    path: &'static str,
    title: &'static str,
    inject_mode: KnowledgeInjectMode,
    maintenance_rules: &'static str,
}

struct MemoryBuiltinDirectorySeed {
    path: &'static str,
    summary: &'static str,
    inject_mode: KnowledgeInjectMode,
    maintenance_rules: &'static str,
}

fn memory_builtin_seeds() -> &'static [MemoryBuiltinSeed] {
    &[
        MemoryBuiltinSeed {
            id: "kd_builtin_memory_project_mistake_note",
            path: MEMORY_PROJECT_MISTAKE_NOTE_PATH,
            title: "Mistake Notebook",
            inject_mode: KnowledgeInjectMode::Full,
            maintenance_rules: MEMORY_PROJECT_MISTAKE_NOTE_RULES,
        },
        MemoryBuiltinSeed {
            id: "kd_builtin_memory_user_preference",
            path: MEMORY_USER_PREFERENCE_PATH,
            title: "User Preferences",
            inject_mode: KnowledgeInjectMode::Rule,
            maintenance_rules: MEMORY_USER_PREFERENCE_RULES,
        },
    ]
}

fn memory_builtin_directory_seeds() -> &'static [MemoryBuiltinDirectorySeed] {
    &[MemoryBuiltinDirectorySeed {
        path: MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        summary: MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY,
        inject_mode: KnowledgeInjectMode::Path,
        maintenance_rules: MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES,
    }]
}

fn memory_document_payload_matches(left: &KnowledgeDocument, right: &KnowledgeDocument) -> bool {
    left.summary.as_deref().unwrap_or("").trim() == right.summary.as_deref().unwrap_or("").trim()
        && left.body.trim() == right.body.trim()
}

fn memory_builtin_documents_match(left: &KnowledgeDocument, right: &KnowledgeDocument) -> bool {
    memory_document_payload_matches(left, right)
        && left.inject_mode == right.inject_mode
        && left
            .maintenance_rules
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            == right
                .maintenance_rules
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
}

fn legacy_memory_builtin_path(seed: &MemoryBuiltinSeed) -> Option<&'static str> {
    match seed.path {
        MEMORY_PROJECT_MISTAKE_NOTE_PATH => Some(MEMORY_PROJECT_MISTAKE_NOTE_LEGACY_PATH),
        MEMORY_USER_PREFERENCE_PATH => Some(MEMORY_USER_PREFERENCE_LEGACY_PATH),
        _ => None,
    }
}

fn resolve_legacy_memory_project_understanding_target(
    working_dir: &str,
    legacy_doc: &KnowledgeDocument,
) -> Result<Option<String>, String> {
    let base_candidates = [
        "unity-project-understanding/overview.md".to_string(),
        "unity-project-understanding/project-understanding.md".to_string(),
        "unity-project-understanding/legacy-project-understanding.md".to_string(),
    ];

    for candidate in base_candidates {
        let target_file = document_path(working_dir, KnowledgeType::Memory, &candidate)?;
        if !target_file.is_file() {
            return Ok(Some(candidate));
        }

        let existing = load_document_by_path(working_dir, KnowledgeType::Memory, &candidate)?;
        if memory_document_payload_matches(legacy_doc, &existing) {
            return Ok(None);
        }
    }

    for index in 2..=32 {
        let candidate = format!(
            "unity-project-understanding/legacy-project-understanding-{}.md",
            index
        );
        let target_file = document_path(working_dir, KnowledgeType::Memory, &candidate)?;
        if !target_file.is_file() {
            return Ok(Some(candidate));
        }

        let existing = load_document_by_path(working_dir, KnowledgeType::Memory, &candidate)?;
        if memory_document_payload_matches(legacy_doc, &existing) {
            return Ok(None);
        }
    }

    Err("Failed to resolve migration target for legacy memory project understanding".to_string())
}

fn migrate_legacy_memory_project_understanding(working_dir: &str) -> Result<(), String> {
    let legacy_path = "project-understanding.md";
    let legacy_file = document_path(working_dir, KnowledgeType::Memory, legacy_path)?;
    if !legacy_file.is_file() {
        return Ok(());
    }

    let legacy_doc = load_document_by_path(working_dir, KnowledgeType::Memory, legacy_path)?;
    let is_legacy_builtin_stub = legacy_doc.id == "kd_builtin_memory_project_understanding"
        && !has_summary_content(legacy_doc.summary.as_deref())
        && !has_body_content(&legacy_doc.body);

    if is_legacy_builtin_stub {
        std::fs::remove_file(&legacy_file).map_err(|e| {
            format!(
                "Failed to delete legacy memory document '{}': {}",
                legacy_file.display(),
                e
            )
        })?;
        return Ok(());
    }

    create_directory(
        working_dir,
        KnowledgeType::Memory,
        "unity-project-understanding",
    )?;
    if let Some(target_path) =
        resolve_legacy_memory_project_understanding_target(working_dir, &legacy_doc)?
    {
        let mut migrated = legacy_doc;
        migrated.path = target_path;
        save_document(working_dir, migrated)?;
    }

    std::fs::remove_file(&legacy_file).map_err(|e| {
        format!(
            "Failed to delete migrated legacy memory document '{}': {}",
            legacy_file.display(),
            e
        )
    })?;

    Ok(())
}

fn memory_builtin_seed_marker_path(working_dir: &str) -> PathBuf {
    Path::new(working_dir)
        .join("Library")
        .join("Locus")
        .join("memory_builtin_seed_version.txt")
}

fn read_memory_builtin_seed_version(working_dir: &str) -> u32 {
    let path = memory_builtin_seed_marker_path(working_dir);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

fn write_memory_builtin_seed_version(working_dir: &str, version: u32) -> Result<(), String> {
    let path = memory_builtin_seed_marker_path(working_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create memory seed marker directory: {}", e))?;
    }
    std::fs::write(&path, version.to_string())
        .map_err(|e| format!("Failed to write memory seed marker: {}", e))
}

fn migrate_builtin_memory_document_rules(
    working_dir: &str,
    seed: &MemoryBuiltinSeed,
    previous_rules: &str,
) -> Result<(), String> {
    let target = document_path(working_dir, KnowledgeType::Memory, seed.path)?;
    if !target.is_file() {
        return Ok(());
    }

    let mut document = load_document_by_path(working_dir, KnowledgeType::Memory, seed.path)?;
    let Some(current_rules) = document.maintenance_rules.as_deref().map(str::trim) else {
        return Ok(());
    };

    if document.id != seed.id || current_rules != previous_rules {
        return Ok(());
    }

    document.maintenance_rules = Some(seed.maintenance_rules.to_string());
    save_document(working_dir, document)?;
    Ok(())
}

fn migrate_builtin_memory_document_inject_mode(
    working_dir: &str,
    seed_id: &str,
    path: &str,
    previous_mode: KnowledgeInjectMode,
    next_mode: KnowledgeInjectMode,
) -> Result<(), String> {
    let target = document_path(working_dir, KnowledgeType::Memory, path)?;
    if !target.is_file() {
        return Ok(());
    }

    let mut document = load_document_by_path(working_dir, KnowledgeType::Memory, path)?;
    if document.id != seed_id || document.inject_mode != previous_mode {
        return Ok(());
    }

    document.inject_mode = next_mode;
    save_document(working_dir, document)?;
    Ok(())
}

fn migrate_builtin_memory_document_paths(working_dir: &str) -> Result<(), String> {
    for seed in memory_builtin_seeds() {
        let Some(legacy_path) = legacy_memory_builtin_path(seed) else {
            continue;
        };
        let legacy_file = document_path(working_dir, KnowledgeType::Memory, legacy_path)?;
        if !legacy_file.is_file() {
            continue;
        }

        let target_file = document_path(working_dir, KnowledgeType::Memory, seed.path)?;
        if !target_file.is_file() {
            let mut legacy_doc =
                load_document_by_path(working_dir, KnowledgeType::Memory, legacy_path)?;
            legacy_doc.path = seed.path.to_string();
            save_document(working_dir, legacy_doc)?;
            std::fs::remove_file(&legacy_file).map_err(|e| {
                format!(
                    "Failed to delete migrated legacy memory document '{}': {}",
                    legacy_file.display(),
                    e
                )
            })?;
            continue;
        }

        let legacy_doc = load_document_by_path(working_dir, KnowledgeType::Memory, legacy_path)?;
        let current_doc = load_document_by_path(working_dir, KnowledgeType::Memory, seed.path)?;
        if memory_builtin_documents_match(&legacy_doc, &current_doc) {
            std::fs::remove_file(&legacy_file).map_err(|e| {
                format!(
                    "Failed to delete duplicate legacy memory document '{}': {}",
                    legacy_file.display(),
                    e
                )
            })?;
        }
    }
    Ok(())
}

fn migrate_memory_builtin_seed_updates(
    working_dir: &str,
    previous_seed_version: u32,
) -> Result<(), String> {
    if previous_seed_version < 5 {
        let record = read_directory_config(
            working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )?;
        if record.exists
            && record.config.maintenance_rules.trim() == MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES_V4
        {
            let mut next = record.config;
            next.summary = MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY.to_string();
            next.maintenance_rules = MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES.to_string();
            update_directory_config(
                working_dir,
                KnowledgeType::Memory,
                MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
                next,
            )?;
        }
    }

    if previous_seed_version < 6 {
        let record = read_directory_config(
            working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )?;
        if record.exists
            && record.config.inject_mode == KnowledgeInjectMode::Excerpt
            && record.config.summary.trim() == MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY
            && record.config.maintenance_rules.trim() == MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES
        {
            let mut next = record.config;
            next.inject_mode = KnowledgeInjectMode::Path;
            update_directory_config(
                working_dir,
                KnowledgeType::Memory,
                MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
                next,
            )?;
        }
    }

    if previous_seed_version < 7 {
        for seed in memory_builtin_seeds() {
            let previous_rules = match seed.path {
                MEMORY_PROJECT_MISTAKE_NOTE_PATH => MEMORY_PROJECT_MISTAKE_NOTE_RULES_V6,
                MEMORY_USER_PREFERENCE_PATH => MEMORY_USER_PREFERENCE_RULES_V6,
                _ => continue,
            };
            migrate_builtin_memory_document_rules(working_dir, seed, previous_rules)?;
        }
    }
    if previous_seed_version < 9 {
        migrate_builtin_memory_document_inject_mode(
            working_dir,
            "kd_builtin_memory_user_preference",
            MEMORY_USER_PREFERENCE_PATH,
            KnowledgeInjectMode::Full,
            KnowledgeInjectMode::Rule,
        )?;
    }
    if previous_seed_version < 10 {
        let record = read_directory_config(
            working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )?;
        if record.exists
            && record.config.maintenance_rules.trim() == MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES_V9
        {
            let mut next = record.config;
            next.maintenance_rules = MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES.to_string();
            update_directory_config(
                working_dir,
                KnowledgeType::Memory,
                MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
                next,
            )?;
        }

        for seed in memory_builtin_seeds() {
            let previous_rules = match seed.path {
                MEMORY_PROJECT_MISTAKE_NOTE_PATH => MEMORY_PROJECT_MISTAKE_NOTE_RULES_V9,
                MEMORY_USER_PREFERENCE_PATH => MEMORY_USER_PREFERENCE_RULES_V9,
                _ => continue,
            };
            migrate_builtin_memory_document_rules(working_dir, seed, previous_rules)?;
        }
    }
    if previous_seed_version < 11 {
        let record = read_directory_config(
            working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )?;
        if record.exists
            && record.config.summary.trim() == MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY_V10
        {
            let mut next = record.config;
            next.summary = MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY.to_string();
            update_directory_config(
                working_dir,
                KnowledgeType::Memory,
                MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
                next,
            )?;
        }
    }
    Ok(())
}

pub fn ensure_memory_builtin_documents(working_dir: &str) -> Result<(), String> {
    if working_dir.trim().is_empty() {
        return Ok(());
    }

    ensure_knowledge_roots(working_dir)?;
    migrate_legacy_memory_project_understanding(working_dir)?;
    let previous_seed_version = read_memory_builtin_seed_version(working_dir);
    if previous_seed_version >= MEMORY_BUILTIN_SEED_VERSION {
        return Ok(());
    }
    if previous_seed_version < 8 {
        migrate_builtin_memory_document_paths(working_dir)?;
    }

    for seed in memory_builtin_directory_seeds() {
        create_directory(working_dir, KnowledgeType::Memory, seed.path)?;
        let (config_file, _) =
            directory_config_path(working_dir, KnowledgeType::Memory, seed.path)?;
        if !config_file.is_file() {
            update_directory_config(
                working_dir,
                KnowledgeType::Memory,
                seed.path,
                KnowledgeDirectoryConfig {
                    version: default_directory_config_version(),
                    summary: seed.summary.to_string(),
                    inject_mode: seed.inject_mode,
                    inherit_inject_mode: false,
                    ai_maintained: true,
                    inherit_ai_config: false,
                    explicit_maintenance_rules: true,
                    lexical_search: FolderIndexRuleSetting::Inherit,
                    vector_search: FolderIndexRuleSetting::Inherit,
                    inherit_to_children: true,
                    allow_create_documents: true,
                    allow_create_directories: true,
                    allow_move_documents: true,
                    allow_move_directories: true,
                    maintenance_rules: seed.maintenance_rules.to_string(),
                },
            )?;
        }
    }

    for seed in memory_builtin_seeds() {
        let target = document_path(working_dir, KnowledgeType::Memory, seed.path)?;
        if target.is_file() {
            continue;
        }

        save_document(
            working_dir,
            KnowledgeDocument {
                id: seed.id.to_string(),
                doc_type: KnowledgeType::Memory,
                path: seed.path.to_string(),
                title: seed.title.to_string(),
                inject_mode: seed.inject_mode,
                inherit_inject_mode: false,
                inject_mode_source: self_config_source(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: true,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: self_config_source(),
                explicit_maintenance_rules: true,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                tools: Vec::new(),
                summary: None,
                body: String::new(),
                maintenance_rules: Some(seed.maintenance_rules.to_string()),
                created_at: now_millis(),
                updated_at: now_millis(),
            },
        )?;
    }

    migrate_memory_builtin_seed_updates(working_dir, previous_seed_version)?;
    write_memory_builtin_seed_version(working_dir, MEMORY_BUILTIN_SEED_VERSION)
}

fn has_summary_content(summary: Option<&str>) -> bool {
    summary
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

pub(crate) fn apply_external_source_defaults(document: &mut KnowledgeDocument) {
    if matches!(
        document
            .external_source
            .as_ref()
            .map(|source| source.provider),
        Some(KnowledgeSourceProvider::Unity)
    ) {
        document.summary_enabled = false;
    }
    if matches!(
        document
            .external_source
            .as_ref()
            .map(|source| source.provider),
        Some(KnowledgeSourceProvider::Feishu)
    ) {
        if let Some(stripped) = strip_legacy_feishu_import_header(&document.body, &document.title) {
            document.body = stripped;
        }
    }
}

fn strip_legacy_feishu_import_header(body: &str, _document_title: &str) -> Option<String> {
    let normalized = body.replace("\r\n", "\n");
    let (prefix, content) = normalized.split_once("\n\n---\n\n")?;
    let mut lines = prefix.lines();

    let title_line = lines.next()?.trim();
    let _ = title_line.strip_prefix("# ")?.trim();
    if lines.next()? != "" {
        return None;
    }
    if !lines.next()?.starts_with("来源：飞书知识库 / ") {
        return None;
    }
    if !lines.next()?.starts_with("节点：`") {
        return None;
    }
    if !lines.next()?.starts_with("对象类型：`") {
        return None;
    }
    if lines.next().is_some() {
        return None;
    }

    Some(content.to_string())
}

pub(crate) fn active_summary(document: &KnowledgeDocument) -> Option<&str> {
    if !document.summary_enabled {
        return None;
    }
    document
        .summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
}

fn has_body_content(body: &str) -> bool {
    !body.trim().is_empty()
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn normalize_tool_names(values: Vec<String>) -> Vec<String> {
    let mut names = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn normalize_command_trigger_value(value: Option<String>) -> Option<String> {
    normalize_optional_text(value).and_then(|item| {
        let trimmed = item.trim_start_matches('/').trim();
        (!trimmed.is_empty()).then(|| format!("/{}", trimmed))
    })
}

fn ensure_summary_state(document: &mut KnowledgeDocument) {
    document.summary = normalize_optional_text(document.summary.take());
}

fn has_maintenance_rules_content(value: Option<&str>) -> bool {
    value.map(|item| !item.trim().is_empty()).unwrap_or(false)
}

pub(crate) fn active_maintenance_rules(document: &KnowledgeDocument) -> Option<&str> {
    if !document.explicit_maintenance_rules {
        return None;
    }
    document
        .maintenance_rules
        .as_deref()
        .filter(|value| !value.trim().is_empty())
}

fn inherited_document_config_and_sources_from_root(
    knowledge_root: Option<&Path>,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<
    (
        KnowledgeDirectoryConfig,
        KnowledgeConfigSource,
        KnowledgeConfigSource,
    ),
    String,
> {
    let Some(knowledge_root) = knowledge_root else {
        return Ok((
            default_document_inherited_config_for_type(doc_type),
            type_default_config_source(),
            type_default_config_source(),
        ));
    };

    let Some(parent_path) = relative_parent_directory(path) else {
        return Ok((
            default_document_inherited_config_for_type(doc_type),
            type_default_config_source(),
            type_default_config_source(),
        ));
    };

    let parent_dir = type_root_in_knowledge_root(knowledge_root, doc_type).join(&parent_path);
    if !parent_dir.is_dir() {
        return Ok((
            default_document_inherited_config_for_type(doc_type),
            type_default_config_source(),
            type_default_config_source(),
        ));
    }

    let parent =
        read_directory_config_from_knowledge_root_internal(knowledge_root, doc_type, &parent_path)?;
    if parent.config.inherit_to_children {
        Ok((
            child_directory_config_from_parent(doc_type, &parent.config),
            inherit_source_from_parent(&parent.inject_mode_source, &parent.path),
            inherit_source_from_parent(&parent.ai_config_source, &parent.path),
        ))
    } else {
        Ok((
            default_document_inherited_config_for_type(doc_type),
            type_default_config_source(),
            type_default_config_source(),
        ))
    }
}

fn resolve_document_inheritance(
    working_dir: Option<&str>,
    document: &mut KnowledgeDocument,
) -> Result<(), String> {
    let knowledge_root = working_dir.map(knowledge_root);
    resolve_document_inheritance_from_root(knowledge_root.as_deref(), document)
}

fn resolve_document_inheritance_from_root(
    knowledge_root: Option<&Path>,
    document: &mut KnowledgeDocument,
) -> Result<(), String> {
    let (inherited_config, inherited_inject_source, inherited_ai_source) =
        inherited_document_config_and_sources_from_root(
            knowledge_root,
            document.doc_type,
            &document.path,
        )?;

    if document.inherit_inject_mode {
        document.inject_mode = inherited_config.inject_mode;
        document.inject_mode_source = inherited_inject_source;
    } else {
        document.inject_mode_source = self_config_source();
    }

    if document.inherit_ai_config {
        document.ai_maintained = inherited_config.ai_maintained;
        document.explicit_maintenance_rules = inherited_config.explicit_maintenance_rules;
        document.maintenance_rules = if inherited_config.explicit_maintenance_rules {
            normalize_optional_text(Some(inherited_config.maintenance_rules.clone()))
        } else {
            None
        };
        document.ai_config_source = inherited_ai_source;
    } else {
        document.ai_config_source = self_config_source();
    }

    Ok(())
}

fn ensure_maintenance_rules(document: &mut KnowledgeDocument) {
    if document.ai_maintained {
        document.explicit_maintenance_rules = true;
    }

    let normalized_rules = normalize_optional_text(document.maintenance_rules.take());
    document.maintenance_rules = normalized_rules;

    if document.ai_maintained
        && !has_maintenance_rules_content(document.maintenance_rules.as_deref())
    {
        document.maintenance_rules =
            default_maintenance_rules_for_type(document.doc_type).map(str::to_string);
    }
}

fn ensure_skill_defaults(document: &mut KnowledgeDocument) {
    if document.doc_type != KnowledgeType::Skill {
        document.skill_enabled = None;
        document.skill_surface = None;
        document.command_trigger = None;
        document.argument_hint = None;
        document.tools.clear();
        return;
    }

    let enabled = document.skill_enabled.unwrap_or(true);
    let surface = document.skill_surface.unwrap_or_default();
    document.skill_enabled = Some(enabled);
    document.skill_surface = Some(surface);
    document.command_trigger = normalize_command_trigger_value(document.command_trigger.take());
    document.argument_hint = normalize_optional_text(document.argument_hint.take());
    document.tools = normalize_tool_names(std::mem::take(&mut document.tools));
    document.command_enabled = enabled && surface.allows_command();
}

fn apply_skill_command_enabled(document: &mut KnowledgeDocument, command_enabled: bool) {
    let current_enabled = document.skill_enabled.unwrap_or(true);
    let current_surface = document.skill_surface.unwrap_or_default();
    let next_surface = if command_enabled {
        if current_surface.allows_auto() {
            SkillSurface::Both
        } else {
            SkillSurface::Command
        }
    } else if current_surface.allows_auto() {
        SkillSurface::Auto
    } else {
        current_surface
    };

    document.skill_enabled = Some(if command_enabled {
        true
    } else {
        current_enabled
    });
    document.skill_surface = Some(next_surface);
}

fn apply_read_only_policy(document: &mut KnowledgeDocument) {
    document.read_only = document.read_only || derive_read_only(document.external_source.as_ref());
}

fn is_read_only_locked_by_source(document: &KnowledgeDocument) -> bool {
    derive_read_only(document.external_source.as_ref())
}

pub fn knowledge_root(working_dir: &str) -> PathBuf {
    Path::new(working_dir).join(KNOWLEDGE_ROOT_DIR)
}

fn type_root_in_knowledge_root(knowledge_root: &Path, doc_type: KnowledgeType) -> PathBuf {
    knowledge_root.join(doc_type.as_str())
}

fn type_root(working_dir: &str, doc_type: KnowledgeType) -> PathBuf {
    knowledge_root(working_dir).join(doc_type.as_str())
}

pub fn ensure_knowledge_roots(working_dir: &str) -> Result<(), String> {
    std::fs::create_dir_all(knowledge_root(working_dir))
        .map_err(|e| format!("Failed to create knowledge root: {}", e))?;
    for doc_type in KnowledgeType::all() {
        std::fs::create_dir_all(type_root(working_dir, doc_type))
            .map_err(|e| format!("Failed to create knowledge type root: {}", e))?;
    }
    migrate_legacy_directory_config_suffixes(working_dir)?;
    Ok(())
}

fn normalize_relative_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty".to_string());
    }
    if trimmed.contains("..") || trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err("Path must be relative to the knowledge root".to_string());
    }

    let normalized = trimmed.replace('\\', "/");
    let normalized = normalized
        .strip_prefix("Locus/knowledge/")
        .unwrap_or(&normalized);
    let normalized = normalized
        .strip_prefix("design/")
        .or_else(|| normalized.strip_prefix("memory/"))
        .or_else(|| normalized.strip_prefix("skill/"))
        .or_else(|| normalized.strip_prefix("reference/"))
        .unwrap_or(normalized);

    Ok(if normalized.ends_with(".md") {
        normalized.to_string()
    } else {
        format!("{}.md", normalized)
    })
}

fn normalize_relative_prefix(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.contains("..") || trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err("Path prefix must be relative to the knowledge root".to_string());
    }

    let normalized = trimmed.replace('\\', "/");
    let normalized = normalized
        .strip_prefix("Locus/knowledge/")
        .unwrap_or(&normalized);
    let normalized = normalized
        .strip_prefix("design/")
        .or_else(|| normalized.strip_prefix("memory/"))
        .or_else(|| normalized.strip_prefix("skill/"))
        .or_else(|| normalized.strip_prefix("reference/"))
        .unwrap_or(&normalized);
    Ok(normalized.trim_end_matches('/').to_string())
}

fn normalize_relative_directory_path(path: &str) -> Result<String, String> {
    let normalized = normalize_relative_prefix(path)?;
    if normalized.is_empty() {
        return Err("Knowledge directory path cannot be empty".to_string());
    }
    Ok(normalized)
}

fn directory_config_file_name_with_suffix(dir_name: &str, suffix: &str) -> Result<String, String> {
    let trimmed = dir_name.trim();
    if trimmed.is_empty() {
        return Err("Knowledge directory name cannot be empty".to_string());
    }
    Ok(format!("{}{}", trimmed, suffix))
}

fn directory_config_path_in_type_root_with_suffix(
    type_root: &Path,
    path: &str,
    suffix: &str,
) -> Result<(PathBuf, String), String> {
    let rel = normalize_relative_directory_path(path)?;
    let rel_path = Path::new(&rel);
    let parent = rel_path.parent().unwrap_or_else(|| Path::new(""));
    let dir_name = rel_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Knowledge directory path is invalid".to_string())?;
    let file_name = directory_config_file_name_with_suffix(dir_name, suffix)?;
    let config_rel = if parent.as_os_str().is_empty() {
        file_name.clone()
    } else {
        format!(
            "{}/{}",
            parent.to_string_lossy().replace('\\', "/"),
            file_name
        )
    };
    Ok((type_root.join(parent).join(file_name), config_rel))
}

fn directory_config_path_in_type_root(
    type_root: &Path,
    path: &str,
) -> Result<(PathBuf, String), String> {
    directory_config_path_in_type_root_with_suffix(
        type_root,
        path,
        KNOWLEDGE_DIRECTORY_CONFIG_SUFFIX,
    )
}

fn legacy_directory_config_path_in_type_root(
    type_root: &Path,
    path: &str,
) -> Result<(PathBuf, String), String> {
    directory_config_path_in_type_root_with_suffix(
        type_root,
        path,
        LEGACY_KNOWLEDGE_DIRECTORY_CONFIG_SUFFIX,
    )
}

fn directory_config_path(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<(PathBuf, String), String> {
    directory_config_path_in_type_root(&type_root(working_dir, doc_type), path)
}

fn is_directory_config_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| {
            value.ends_with(KNOWLEDGE_DIRECTORY_CONFIG_SUFFIX)
                || value.ends_with(LEGACY_KNOWLEDGE_DIRECTORY_CONFIG_SUFFIX)
        })
        .unwrap_or(false)
}

fn directory_path_from_config_file(type_root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(type_root)
        .map_err(|e| format!("Failed to resolve directory config path: {}", e))?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let file_name = relative
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Knowledge directory config file name is invalid".to_string())?;
    let dir_name = if let Some(value) = file_name.strip_suffix(KNOWLEDGE_DIRECTORY_CONFIG_SUFFIX) {
        value
    } else if let Some(value) = file_name.strip_suffix(LEGACY_KNOWLEDGE_DIRECTORY_CONFIG_SUFFIX) {
        value
    } else {
        return Err(format!(
            "Knowledge directory config file has an unsupported suffix: {}",
            path.display()
        ));
    };
    let dir_path = if parent.as_os_str().is_empty() {
        dir_name.to_string()
    } else {
        format!(
            "{}/{}",
            parent.to_string_lossy().replace('\\', "/"),
            dir_name
        )
    };
    normalize_relative_directory_path(&dir_path)
}

fn cleanup_duplicate_legacy_directory_config(
    legacy_path: &Path,
    current_path: &Path,
) -> Result<(), String> {
    if !legacy_path.is_file() || !current_path.is_file() {
        return Ok(());
    }

    let Ok(legacy_raw) = std::fs::read(legacy_path) else {
        return Ok(());
    };
    let Ok(current_raw) = std::fs::read(current_path) else {
        return Ok(());
    };
    if legacy_raw != current_raw {
        return Ok(());
    }

    std::fs::remove_file(legacy_path).map_err(|e| {
        format!(
            "Failed to delete duplicate legacy knowledge directory config '{}': {}",
            legacy_path.display(),
            e
        )
    })
}

fn migrate_legacy_directory_config_for_path(type_root: &Path, path: &str) -> Result<(), String> {
    let (current_path, _) = directory_config_path_in_type_root(type_root, path)?;
    let (legacy_path, _) = legacy_directory_config_path_in_type_root(type_root, path)?;

    if current_path.is_file() {
        cleanup_duplicate_legacy_directory_config(&legacy_path, &current_path)?;
        return Ok(());
    }
    if !legacy_path.is_file() {
        return Ok(());
    }

    if let Some(parent) = current_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create migrated knowledge directory config parent '{}': {}",
                parent.display(),
                e
            )
        })?;
    }

    std::fs::rename(&legacy_path, &current_path).map_err(|e| {
        format!(
            "Failed to migrate knowledge directory config '{}' -> '{}': {}",
            legacy_path.display(),
            current_path.display(),
            e
        )
    })?;
    Ok(())
}

fn migrate_legacy_directory_config_suffixes(working_dir: &str) -> Result<(), String> {
    let knowledge_root = knowledge_root(working_dir);
    if !knowledge_root.is_dir() {
        return Ok(());
    }

    for doc_type in KnowledgeType::all() {
        let type_root = knowledge_root.join(doc_type.as_str());
        if !type_root.is_dir() {
            continue;
        }

        for entry in WalkDir::new(&type_root).min_depth(1).into_iter().flatten() {
            if !entry.file_type().is_dir() {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(&type_root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if relative.is_empty() {
                continue;
            }
            migrate_legacy_directory_config_for_path(&type_root, &relative)?;
        }
    }

    Ok(())
}

fn normalize_directory_config_text(value: String) -> String {
    value.replace("\r\n", "\n").trim().to_string()
}

fn normalize_optional_directory_config_text(value: Option<String>) -> Option<String> {
    value
        .map(normalize_directory_config_text)
        .filter(|item| !item.is_empty())
}

fn normalize_stored_directory_config(
    config: &mut StoredKnowledgeDirectoryConfig,
    doc_type: KnowledgeType,
) {
    let legacy_version = config.version;
    config.summary = normalize_directory_config_text(std::mem::take(&mut config.summary));
    config.maintenance_rules =
        normalize_optional_directory_config_text(std::mem::take(&mut config.maintenance_rules));

    let inherit_ai_config = config.inherit_ai_config.unwrap_or(false);
    let has_rules = config
        .maintenance_rules
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    if !inherit_ai_config {
        if legacy_version < 2 && config.explicit_maintenance_rules.is_none() {
            config.explicit_maintenance_rules = Some(
                default_explicit_maintenance_rules_for_type(doc_type)
                    || config.ai_maintained.unwrap_or(false)
                    || has_rules,
            );
        } else if config.ai_maintained.unwrap_or(false) || has_rules {
            config.explicit_maintenance_rules = Some(true);
        }
        if config.explicit_maintenance_rules == Some(false) {
            config.maintenance_rules = None;
        }
    } else {
        config.ai_maintained = None;
        config.explicit_maintenance_rules = None;
        config.maintenance_rules = None;
    }

    if legacy_version < 3
        && config.inject_mode.is_none()
        && !config.inherit_inject_mode.unwrap_or(false)
    {
        config.inject_mode = Some(default_directory_inject_mode());
    }

    config.version = default_directory_config_version();
}

fn normalize_directory_config(config: &mut KnowledgeDirectoryConfig, _doc_type: KnowledgeType) {
    config.summary = normalize_directory_config_text(std::mem::take(&mut config.summary));
    config.maintenance_rules =
        normalize_directory_config_text(std::mem::take(&mut config.maintenance_rules));
    let has_rules = !config.maintenance_rules.trim().is_empty();
    if config.ai_maintained || has_rules {
        config.explicit_maintenance_rules = true;
    }
    if !config.explicit_maintenance_rules {
        config.maintenance_rules.clear();
    }
    if config.inherit_ai_config && !config.explicit_maintenance_rules {
        config.maintenance_rules.clear();
    }
    config.version = default_directory_config_version();
}

fn relative_parent_directory(path: &str) -> Option<String> {
    let parent = Path::new(path).parent()?;
    let normalized = parent.to_string_lossy().replace('\\', "/");
    let trimmed = normalized.trim_matches('/').trim();
    if trimmed.is_empty() || trimmed == "." {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn child_directory_config_from_parent(
    doc_type: KnowledgeType,
    parent: &KnowledgeDirectoryConfig,
) -> KnowledgeDirectoryConfig {
    let mut config = KnowledgeDirectoryConfig {
        version: default_directory_config_version(),
        summary: String::new(),
        inject_mode: parent.inject_mode,
        inherit_inject_mode: true,
        ai_maintained: parent.ai_maintained,
        inherit_ai_config: true,
        explicit_maintenance_rules: parent.explicit_maintenance_rules,
        lexical_search: FolderIndexRuleSetting::Inherit,
        vector_search: FolderIndexRuleSetting::Inherit,
        inherit_to_children: parent.inherit_to_children,
        allow_create_documents: parent.allow_create_documents,
        allow_create_directories: parent.allow_create_directories,
        allow_move_documents: parent.allow_move_documents,
        allow_move_directories: parent.allow_move_directories,
        maintenance_rules: parent.maintenance_rules.clone(),
    };
    normalize_directory_config(&mut config, doc_type);
    config
}

fn validate_directory_config(config: &KnowledgeDirectoryConfig) -> Result<(), String> {
    if matches!(
        config.inject_mode,
        KnowledgeInjectMode::Full | KnowledgeInjectMode::Rule
    ) {
        return Err(
            "Directory config does not support injectMode=full or injectMode=rule".to_string(),
        );
    }
    if config.ai_maintained && !config.explicit_maintenance_rules {
        return Err(
            "Directory config with aiMaintained=true requires explicitMaintenanceRules=true"
                .to_string(),
        );
    }
    if config.ai_maintained && config.maintenance_rules.trim().is_empty() {
        return Err(
            "Directory config with aiMaintained=true requires maintenance rules".to_string(),
        );
    }
    Ok(())
}

fn stored_directory_config_from_runtime_config(
    config: &KnowledgeDirectoryConfig,
    external_sources: Vec<KnowledgeExternalSource>,
) -> StoredKnowledgeDirectoryConfig {
    StoredKnowledgeDirectoryConfig {
        version: config.version,
        summary: config.summary.clone(),
        inject_mode: (!config.inherit_inject_mode).then_some(config.inject_mode),
        inherit_inject_mode: config.inherit_inject_mode.then_some(true),
        ai_maintained: (!config.inherit_ai_config).then_some(config.ai_maintained),
        inherit_ai_config: config.inherit_ai_config.then_some(true),
        explicit_maintenance_rules: (!config.inherit_ai_config)
            .then_some(config.explicit_maintenance_rules),
        lexical_search: config.lexical_search,
        vector_search: config.vector_search,
        inherit_to_children: config.inherit_to_children,
        allow_create_documents: config.allow_create_documents,
        allow_create_directories: config.allow_create_directories,
        allow_move_documents: config.allow_move_documents,
        allow_move_directories: config.allow_move_directories,
        maintenance_rules: (!config.inherit_ai_config && config.explicit_maintenance_rules)
            .then_some(config.maintenance_rules.clone()),
        external_sources,
    }
}

fn render_directory_config(
    config: &KnowledgeDirectoryConfig,
    external_sources: Vec<KnowledgeExternalSource>,
) -> Result<String, String> {
    let stored = stored_directory_config_from_runtime_config(config, external_sources);
    let mut raw = serde_yaml::to_string(&stored)
        .map_err(|e| format!("Failed to serialize knowledge directory config: {}", e))?;
    if let Some(stripped) = raw.strip_prefix("---\n") {
        raw = stripped.to_string();
    }
    if !raw.ends_with('\n') {
        raw.push('\n');
    }
    Ok(raw)
}

pub fn document_path(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<PathBuf, String> {
    let rel = normalize_relative_path(path)?;
    Ok(type_root(working_dir, doc_type).join(rel))
}

pub fn document_path_in_root(
    root: &Path,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<PathBuf, String> {
    let rel = normalize_relative_path(path)?;
    Ok(root.join(doc_type.as_str()).join(rel))
}

fn inherit_source_from_parent(
    source: &KnowledgeConfigSource,
    parent_path: &str,
) -> KnowledgeConfigSource {
    match source.kind {
        KnowledgeConfigSourceKind::SelfValue => parent_directory_config_source(parent_path),
        _ => source.clone(),
    }
}

fn inherited_directory_config_and_sources(
    doc_type: KnowledgeType,
    parent: Option<&KnowledgeDirectoryConfigRecord>,
) -> (
    KnowledgeDirectoryConfig,
    KnowledgeConfigSource,
    KnowledgeConfigSource,
) {
    if let Some(parent_record) = parent.filter(|record| record.config.inherit_to_children) {
        return (
            child_directory_config_from_parent(doc_type, &parent_record.config),
            inherit_source_from_parent(&parent_record.inject_mode_source, &parent_record.path),
            inherit_source_from_parent(&parent_record.ai_config_source, &parent_record.path),
        );
    }

    (
        default_directory_config_for_type(doc_type),
        type_default_config_source(),
        type_default_config_source(),
    )
}

fn inherited_capability_source_dir(
    parent: &KnowledgeDirectoryConfigRecord,
    state: &EffectiveCapabilityState,
) -> Option<String> {
    if let Some(path) = state
        .source_dir
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        return Some(path.clone());
    }
    match state.source.as_str() {
        "self" | "parent" => Some(parent.path.clone()),
        _ => None,
    }
}

fn resolve_effective_capability_state(
    setting: FolderIndexRuleSetting,
    parent: Option<&KnowledgeDirectoryConfigRecord>,
    parent_effective: fn(&KnowledgeDirectoryConfigRecord) -> &EffectiveCapabilityState,
) -> EffectiveCapabilityState {
    match setting {
        FolderIndexRuleSetting::Enabled => self_capability_state(true),
        FolderIndexRuleSetting::Disabled => self_capability_state(false),
        FolderIndexRuleSetting::Inherit => {
            if let Some(parent_record) = parent.filter(|record| record.config.inherit_to_children) {
                let parent_state = parent_effective(parent_record);
                return EffectiveCapabilityState {
                    enabled: parent_state.enabled,
                    source: "parent".to_string(),
                    reason_code: parent_state.reason_code.clone(),
                    source_dir: inherited_capability_source_dir(parent_record, parent_state),
                };
            }
            default_enabled_capability_state()
        }
    }
}

fn directory_search_access_from_record(
    record: &KnowledgeDirectoryConfigRecord,
) -> DirectorySearchAccess {
    DirectorySearchAccess {
        lexical_enabled: record.effective_lexical_search.enabled,
        vector_enabled: record.effective_vector_search.enabled,
    }
}

fn resolve_directory_config_record(
    doc_type: KnowledgeType,
    dir_path: &str,
    config_path: String,
    exists: bool,
    updated_at: i64,
    stored: Option<StoredKnowledgeDirectoryConfig>,
    parent: Option<&KnowledgeDirectoryConfigRecord>,
) -> Result<KnowledgeDirectoryConfigRecord, String> {
    let (inherited_config, inherited_inject_source, inherited_ai_source) =
        inherited_directory_config_and_sources(doc_type, parent);

    let (mut config, inject_mode_source, ai_config_source, external_sources) =
        if let Some(mut stored) = stored {
            let external_sources = std::mem::take(&mut stored.external_sources);
            normalize_stored_directory_config(&mut stored, doc_type);
            let inherit_inject_mode = stored.inherit_inject_mode.unwrap_or(false);
            let inherit_ai_config = stored.inherit_ai_config.unwrap_or(false);
            (
                KnowledgeDirectoryConfig {
                    version: stored.version,
                    summary: stored.summary,
                    inject_mode: if inherit_inject_mode {
                        inherited_config.inject_mode
                    } else {
                        stored
                            .inject_mode
                            .unwrap_or_else(default_directory_inject_mode)
                    },
                    inherit_inject_mode,
                    ai_maintained: if inherit_ai_config {
                        inherited_config.ai_maintained
                    } else {
                        stored.ai_maintained.unwrap_or(false)
                    },
                    inherit_ai_config,
                    explicit_maintenance_rules: if inherit_ai_config {
                        inherited_config.explicit_maintenance_rules
                    } else {
                        stored.explicit_maintenance_rules.unwrap_or(false)
                    },
                    lexical_search: stored.lexical_search,
                    vector_search: stored.vector_search,
                    inherit_to_children: stored.inherit_to_children,
                    allow_create_documents: stored.allow_create_documents,
                    allow_create_directories: stored.allow_create_directories,
                    allow_move_documents: stored.allow_move_documents,
                    allow_move_directories: stored.allow_move_directories,
                    maintenance_rules: if inherit_ai_config {
                        inherited_config.maintenance_rules.clone()
                    } else {
                        stored.maintenance_rules.unwrap_or_default()
                    },
                },
                if inherit_inject_mode {
                    inherited_inject_source
                } else {
                    self_config_source()
                },
                if inherit_ai_config {
                    inherited_ai_source
                } else {
                    self_config_source()
                },
                external_sources,
            )
        } else {
            let mut config = inherited_config;
            config.inherit_inject_mode = true;
            config.inherit_ai_config = true;
            (
                config,
                inherited_inject_source,
                inherited_ai_source,
                Vec::new(),
            )
        };

    normalize_directory_config(&mut config, doc_type);
    validate_directory_config(&config)?;
    let effective_lexical_search =
        resolve_effective_capability_state(config.lexical_search, parent, |record| {
            &record.effective_lexical_search
        });
    let effective_vector_search =
        resolve_effective_capability_state(config.vector_search, parent, |record| {
            &record.effective_vector_search
        });

    Ok(KnowledgeDirectoryConfigRecord {
        doc_type,
        path: dir_path.to_string(),
        config_path,
        exists,
        read_only: false,
        updated_at,
        inject_mode_source,
        ai_config_source,
        effective_lexical_search,
        effective_vector_search,
        external_sources,
        config,
    })
}

fn read_stored_directory_config(
    config_file: &Path,
) -> Result<StoredKnowledgeDirectoryConfig, String> {
    let raw = std::fs::read_to_string(config_file).map_err(|e| {
        format!(
            "Failed to read knowledge directory config '{}': {}",
            config_file.display(),
            e
        )
    })?;
    serde_yaml::from_str::<StoredKnowledgeDirectoryConfig>(&raw).map_err(|e| {
        format!(
            "Failed to parse knowledge directory config '{}': {}",
            config_file.display(),
            e
        )
    })
}

fn read_directory_config_from_knowledge_root_internal(
    knowledge_root: &Path,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<KnowledgeDirectoryConfigRecord, String> {
    let dir_path = normalize_relative_directory_path(path)?;
    let type_root = type_root_in_knowledge_root(knowledge_root, doc_type);
    let target_dir = type_root.join(&dir_path);
    if !target_dir.is_dir() {
        return Err(format!("Knowledge directory not found: {}", dir_path));
    }

    let (config_file, config_rel) = directory_config_path_in_type_root(&type_root, &dir_path)?;
    let exists = config_file.is_file();
    let stored = if exists {
        Some(read_stored_directory_config(&config_file)?)
    } else {
        None
    };
    let parent_config = relative_parent_directory(&dir_path)
        .as_deref()
        .map(|parent_path| {
            read_directory_config_from_knowledge_root_internal(
                knowledge_root,
                doc_type,
                parent_path,
            )
        })
        .transpose()?;

    let updated_at = if exists {
        std::fs::metadata(&config_file)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0)
    } else {
        0
    };

    resolve_directory_config_record(
        doc_type,
        &dir_path,
        config_rel,
        exists,
        updated_at,
        stored,
        parent_config.as_ref(),
    )
}

fn read_virtual_workspace_directory_config(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<KnowledgeDirectoryConfigRecord, String> {
    let dir_path = normalize_relative_directory_path(path)?;
    let (config_file, config_rel) = directory_config_path(working_dir, doc_type, &dir_path)?;
    let exists = config_file.is_file();
    let stored = if exists {
        Some(read_stored_directory_config(&config_file)?)
    } else {
        None
    };
    let parent_config = relative_parent_directory(&dir_path)
        .as_deref()
        .map(|parent_path| read_directory_config(working_dir, doc_type, parent_path))
        .transpose()?;
    let updated_at = if exists {
        std::fs::metadata(&config_file)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0)
    } else {
        0
    };

    resolve_directory_config_record(
        doc_type,
        &dir_path,
        config_rel,
        exists,
        updated_at,
        stored,
        parent_config.as_ref(),
    )
}

pub fn directory_exists(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<bool, String> {
    let normalized_path = normalize_relative_directory_path(path)?;
    if type_root(working_dir, doc_type)
        .join(&normalized_path)
        .is_dir()
    {
        return Ok(true);
    }
    if doc_type == KnowledgeType::Reference {
        return crate::unity_docs::managed_directory_exists(working_dir, &normalized_path);
    }
    Ok(false)
}

pub fn read_directory_config(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<KnowledgeDirectoryConfigRecord, String> {
    ensure_knowledge_roots(working_dir)?;
    if doc_type == KnowledgeType::Reference
        && crate::unity_docs::managed_directory_exists(working_dir, path)?
    {
        return read_virtual_workspace_directory_config(working_dir, doc_type, path);
    }
    read_directory_config_from_knowledge_root_internal(&knowledge_root(working_dir), doc_type, path)
}

pub fn read_directory_config_from_root(
    knowledge_root: &Path,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<KnowledgeDirectoryConfigRecord, String> {
    read_directory_config_from_knowledge_root_internal(knowledge_root, doc_type, path)
}

pub fn effective_child_directory_config(
    working_dir: &str,
    doc_type: KnowledgeType,
    parent_path: Option<&str>,
) -> Result<KnowledgeDirectoryConfig, String> {
    let Some(parent_path) = parent_path.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(default_directory_config_for_type(doc_type));
    };

    if !directory_exists(working_dir, doc_type, parent_path)? {
        return Ok(default_directory_config_for_type(doc_type));
    }

    let parent = read_directory_config(working_dir, doc_type, parent_path)?;
    if parent.config.inherit_to_children {
        Ok(child_directory_config_from_parent(doc_type, &parent.config))
    } else {
        Ok(default_directory_config_for_type(doc_type))
    }
}

fn default_directory_search_access() -> DirectorySearchAccess {
    DirectorySearchAccess {
        lexical_enabled: true,
        vector_enabled: true,
    }
}

pub fn effective_directory_search_access_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<DirectorySearchAccess, String> {
    let normalized_path = normalize_relative_directory_path(path)?;
    if directory_exists(working_dir, doc_type, &normalized_path)? {
        let record = read_directory_config(working_dir, doc_type, &normalized_path)?;
        return Ok(directory_search_access_from_record(&record));
    }

    if let Some(app_root) = app_knowledge_dir {
        let app_dir = type_root_in_knowledge_root(app_root, doc_type).join(&normalized_path);
        if app_dir.is_dir() {
            let record = read_directory_config_from_root(app_root, doc_type, &normalized_path)?;
            return Ok(directory_search_access_from_record(&record));
        }
    }

    Ok(default_directory_search_access())
}

pub fn effective_document_search_access_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<DirectorySearchAccess, String> {
    let Some(parent_path) = relative_parent_directory(path) else {
        return Ok(default_directory_search_access());
    };
    effective_directory_search_access_with_app_root(
        working_dir,
        app_knowledge_dir,
        doc_type,
        &parent_path,
    )
}

pub fn default_document_title_from_path(path: &str) -> Result<String, String> {
    let normalized = normalize_relative_path(path)?;
    Path::new(&normalized)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .ok_or_else(|| format!("Knowledge document path is invalid: {}", normalized))
}

fn sync_title_with_document_path(doc: &mut KnowledgeDocument) -> Result<(), String> {
    doc.title = default_document_title_from_path(&doc.path)?;
    Ok(())
}

fn sync_title_after_path_change(doc: &mut KnowledgeDocument, new_path: &str) -> Result<(), String> {
    doc.path = normalize_relative_path(new_path)?;
    sync_title_with_document_path(doc)?;
    Ok(())
}

pub fn default_document_create_patch(
    working_dir: &str,
    _doc_type: KnowledgeType,
    path: &str,
) -> Result<KnowledgeDocumentPatch, String> {
    let normalized_path = normalize_relative_path(path)?;
    let _ = working_dir;

    Ok(KnowledgeDocumentPatch {
        title: Some(default_document_title_from_path(&normalized_path)?),
        body: Some(Some(String::new())),
        inherit_inject_mode: Some(true),
        inherit_ai_config: Some(true),
        ..Default::default()
    })
}

pub fn create_directory(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<String, String> {
    ensure_knowledge_roots(working_dir)?;

    let relative_path = normalize_relative_directory_path(path)?;
    if doc_type == KnowledgeType::Reference
        && crate::unity_docs::is_unity_reference_managed_relative_path(&relative_path)
    {
        return Err("Unity 托管参考目录由导入流程维护。".to_string());
    }
    let target = knowledge_root(working_dir)
        .join(doc_type.as_str())
        .join(&relative_path);
    std::fs::create_dir_all(&target).map_err(|e| {
        format!(
            "Failed to create knowledge directory '{}': {}",
            target.display(),
            e
        )
    })?;
    Ok(relative_path)
}

pub fn merge_directory_config_patch(
    mut config: KnowledgeDirectoryConfig,
    patch: &KnowledgeDirectoryConfigPatch,
) -> KnowledgeDirectoryConfig {
    if let Some(version) = patch.version {
        config.version = version;
    }
    if let Some(summary) = patch.summary.as_ref() {
        config.summary = summary.clone();
    }
    if let Some(inherit_inject_mode) = patch.inherit_inject_mode {
        config.inherit_inject_mode = inherit_inject_mode;
    } else if patch.inject_mode.is_some() {
        config.inherit_inject_mode = false;
    }
    if let Some(inject_mode) = patch.inject_mode {
        config.inject_mode = inject_mode;
    }
    if let Some(inherit_ai_config) = patch.inherit_ai_config {
        config.inherit_ai_config = inherit_ai_config;
    } else if patch.ai_maintained.is_some()
        || patch.explicit_maintenance_rules.is_some()
        || patch.maintenance_rules.is_some()
    {
        config.inherit_ai_config = false;
    }
    if let Some(ai_maintained) = patch.ai_maintained {
        config.ai_maintained = ai_maintained;
    }
    if let Some(explicit_maintenance_rules) = patch.explicit_maintenance_rules {
        config.explicit_maintenance_rules = explicit_maintenance_rules;
    }
    if let Some(lexical_search) = patch.lexical_search {
        config.lexical_search = lexical_search;
    }
    if let Some(vector_search) = patch.vector_search {
        config.vector_search = vector_search;
    }
    if let Some(inherit_to_children) = patch.inherit_to_children {
        config.inherit_to_children = inherit_to_children;
    }
    if let Some(allow_create_documents) = patch.allow_create_documents {
        config.allow_create_documents = allow_create_documents;
    }
    if let Some(allow_create_directories) = patch.allow_create_directories {
        config.allow_create_directories = allow_create_directories;
    }
    if let Some(allow_move_documents) = patch.allow_move_documents {
        config.allow_move_documents = allow_move_documents;
    }
    if let Some(allow_move_directories) = patch.allow_move_directories {
        config.allow_move_directories = allow_move_directories;
    }
    if let Some(maintenance_rules) = patch.maintenance_rules.as_ref() {
        config.maintenance_rules = maintenance_rules.clone();
    }
    config
}

fn write_directory_config_record(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
    config: KnowledgeDirectoryConfig,
    external_sources: Vec<KnowledgeExternalSource>,
) -> Result<KnowledgeDirectoryConfigRecord, String> {
    ensure_knowledge_roots(working_dir)?;

    let dir_path = normalize_relative_directory_path(path)?;
    let target_dir = type_root(working_dir, doc_type).join(&dir_path);
    let managed_virtual_directory = doc_type == KnowledgeType::Reference
        && crate::unity_docs::managed_directory_exists(working_dir, &dir_path)?;
    if !target_dir.is_dir() && !managed_virtual_directory {
        return Err(format!("Knowledge directory not found: {}", dir_path));
    }
    if managed_virtual_directory && dir_path != crate::unity_docs::UNITY_REFERENCE_MANAGED_DIR {
        return Err(
            "Unity 托管虚拟子目录继承根目录配置。请编辑 reference/unity-official-docs。"
                .to_string(),
        );
    }

    let (config_file, config_rel) = directory_config_path(working_dir, doc_type, &dir_path)?;
    if let Some(parent) = config_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create knowledge directory config parent '{}': {}",
                parent.display(),
                e
            )
        })?;
    }

    let mut normalized = config;
    normalize_directory_config(&mut normalized, doc_type);
    validate_directory_config(&normalized)?;

    let raw = render_directory_config(&normalized, external_sources.clone())?;
    std::fs::write(&config_file, raw).map_err(|e| {
        format!(
            "Failed to write knowledge directory config '{}': {}",
            config_file.display(),
            e
        )
    })?;

    let updated_at = std::fs::metadata(&config_file)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_else(now_millis);
    let parent_config = relative_parent_directory(&dir_path)
        .as_deref()
        .map(|parent_path| read_directory_config(working_dir, doc_type, parent_path))
        .transpose()?;
    let (_, inject_mode_source, ai_config_source) =
        inherited_directory_config_and_sources(doc_type, parent_config.as_ref());
    let inject_mode_source = if normalized.inherit_inject_mode {
        inject_mode_source
    } else {
        self_config_source()
    };
    let ai_config_source = if normalized.inherit_ai_config {
        ai_config_source
    } else {
        self_config_source()
    };
    let effective_lexical_search = resolve_effective_capability_state(
        normalized.lexical_search,
        parent_config.as_ref(),
        |record| &record.effective_lexical_search,
    );
    let effective_vector_search = resolve_effective_capability_state(
        normalized.vector_search,
        parent_config.as_ref(),
        |record| &record.effective_vector_search,
    );

    Ok(KnowledgeDirectoryConfigRecord {
        doc_type,
        path: dir_path.clone(),
        config_path: config_rel,
        exists: true,
        read_only: false,
        updated_at,
        inject_mode_source,
        ai_config_source,
        effective_lexical_search,
        effective_vector_search,
        external_sources,
        config: normalized,
    })
}

pub fn update_directory_config(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
    config: KnowledgeDirectoryConfig,
) -> Result<KnowledgeDirectoryConfigRecord, String> {
    let dir_path = normalize_relative_directory_path(path)?;
    let (config_file, _) = directory_config_path(working_dir, doc_type, &dir_path)?;
    let external_sources = if config_file.is_file() {
        read_stored_directory_config(&config_file)?.external_sources
    } else {
        Vec::new()
    };
    write_directory_config_record(working_dir, doc_type, &dir_path, config, external_sources)
}

pub fn update_directory_external_sources(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
    external_sources: Vec<KnowledgeExternalSource>,
) -> Result<KnowledgeDirectoryConfigRecord, String> {
    if doc_type != KnowledgeType::Reference {
        return Err("Directory external sources are only supported for reference".to_string());
    }
    let existing = read_directory_config(working_dir, doc_type, path)?;
    write_directory_config_record(
        working_dir,
        doc_type,
        &existing.path,
        existing.config,
        external_sources,
    )
}

pub fn edit_document(
    working_dir: &str,
    path: &str,
    doc_type_hint: Option<KnowledgeType>,
    patch: KnowledgeDocumentPatch,
) -> Result<KnowledgeDocument, String> {
    ensure_knowledge_roots(working_dir)?;

    let mut doc = locate_document(
        working_dir,
        &KnowledgeUpdateRequest {
            op: KnowledgeUpdateOp::Edit,
            path: path.to_string(),
            doc_type: doc_type_hint,
            id: patch.id.clone(),
            ..Default::default()
        },
    )?;

    if doc.read_only && (is_read_only_locked_by_source(&doc) || patch.read_only != Some(false)) {
        return Err("Cannot update a read-only knowledge document".to_string());
    }

    let old_type = doc.doc_type;
    let old_path = doc.path.clone();

    if let Some(doc_type) = patch.doc_type {
        doc.doc_type = doc_type;
    }
    if let Some(inject_mode) = patch.inject_mode {
        doc.inject_mode = inject_mode;
    }
    if let Some(inherit_inject_mode) = patch.inherit_inject_mode {
        doc.inherit_inject_mode = inherit_inject_mode;
    } else if patch.inject_mode.is_some() {
        doc.inherit_inject_mode = false;
    }
    if let Some(summary_enabled) = patch.summary_enabled {
        doc.summary_enabled = summary_enabled;
    }
    if let Some(command_enabled) = patch.command_enabled {
        if doc.doc_type == KnowledgeType::Skill
            && patch.skill_enabled.is_none()
            && patch.skill_surface.is_none()
        {
            apply_skill_command_enabled(&mut doc, command_enabled);
        } else {
            doc.command_enabled = command_enabled;
        }
    }
    if let Some(read_only) = patch.read_only {
        doc.read_only = read_only;
    }
    if let Some(ai_maintained) = patch.ai_maintained {
        doc.ai_maintained = ai_maintained;
    }
    if let Some(inherit_ai_config) = patch.inherit_ai_config {
        doc.inherit_ai_config = inherit_ai_config;
    } else if patch.ai_maintained.is_some()
        || patch.explicit_maintenance_rules.is_some()
        || patch.maintenance_rules.is_some()
    {
        doc.inherit_ai_config = false;
    }
    if let Some(explicit_maintenance_rules) = patch.explicit_maintenance_rules {
        doc.explicit_maintenance_rules = explicit_maintenance_rules;
    }
    if let Some(external_source) = patch.external_source {
        doc.external_source = external_source;
    }
    if let Some(skill_enabled) = patch.skill_enabled {
        doc.skill_enabled = Some(skill_enabled);
    }
    if let Some(skill_surface) = patch.skill_surface {
        doc.skill_surface = Some(skill_surface);
    }
    if let Some(command_trigger) = patch.command_trigger {
        doc.command_trigger = command_trigger;
    }
    if let Some(argument_hint) = patch.argument_hint {
        doc.argument_hint = argument_hint;
    }
    if let Some(summary) = patch.summary {
        doc.summary = summary;
    }
    if let Some(body) = patch.body {
        doc.body = body.ok_or_else(|| "knowledge_edit document body cannot be null".to_string())?;
    }
    if let Some(maintenance_rules) = patch.maintenance_rules {
        doc.maintenance_rules = maintenance_rules;
    }
    if let Some(new_path) = patch.new_path {
        sync_title_after_path_change(&mut doc, &new_path)?;
    }
    if let Some(id) = patch.id {
        doc.id = id;
    }
    ensure_summary_state(&mut doc);
    ensure_maintenance_rules(&mut doc);
    ensure_skill_defaults(&mut doc);
    let saved = save_document(working_dir, doc)?;
    if saved.doc_type != old_type || saved.path != old_path {
        let old_file = document_path(working_dir, old_type, &old_path)?;
        if old_file.is_file() {
            let _ = std::fs::remove_file(old_file);
        }
    }
    Ok(saved)
}

pub fn guess_type_from_path(path: &str) -> Option<KnowledgeType> {
    let normalized = path.replace('\\', "/");
    if normalized.starts_with("design/") {
        Some(KnowledgeType::Design)
    } else if normalized.starts_with("memory/") {
        Some(KnowledgeType::Memory)
    } else if normalized.starts_with("skill/") {
        Some(KnowledgeType::Skill)
    } else if normalized.starts_with("reference/") {
        Some(KnowledgeType::Reference)
    } else {
        None
    }
}

fn split_frontmatter(content: &str) -> Result<(&str, &str), String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err("Knowledge document frontmatter is missing".to_string());
    }

    let after_open = &trimmed[3..];
    let Some(end) = after_open.find("\n---") else {
        return Err("Knowledge document frontmatter is not terminated".to_string());
    };

    let yaml = &after_open[..end];
    let consumed = 3 + end + 4;
    let body = &trimmed[consumed..];
    Ok((yaml, body))
}

fn parse_frontmatter(content: &str) -> Result<(KnowledgeFrontmatter, &str), String> {
    let (yaml, body) = split_frontmatter(content)?;
    let frontmatter: KnowledgeFrontmatter = serde_yaml::from_str(yaml)
        .map_err(|e| format!("Failed to parse knowledge frontmatter: {}", e))?;
    if frontmatter.path.trim().is_empty() {
        return Err("Knowledge frontmatter path is required".to_string());
    }
    if frontmatter.title.trim().is_empty() {
        return Err("Knowledge frontmatter title is required".to_string());
    }
    Ok((frontmatter, body))
}

#[derive(Debug, Clone, Default)]
struct ParsedSections {
    title: Option<String>,
    summary: Option<String>,
    maintenance_rules: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct MemoryCommentSections {
    maintenance_rules: Option<String>,
    body: String,
}

const MEMORY_MAINTAIN_RULES_START: &str = "<!-- locus:maintain-rules:start -->";
const MEMORY_MAINTAIN_RULES_END: &str = "<!-- locus:maintain-rules:end -->";
const MEMORY_BODY_START: &str = "<!-- locus:body:start -->";
const MEMORY_BODY_END: &str = "<!-- locus:body:end -->";

fn finish_section(sections: &mut ParsedSections, name: &'static str, content: &str) {
    let trimmed = content.trim().to_string();
    match name {
        "summary" => {
            if !trimmed.is_empty() {
                sections.summary = Some(trimmed);
            }
        }
        "rules" => {
            if !trimmed.is_empty() {
                sections.maintenance_rules = Some(trimmed);
            }
        }
        "content" => {
            sections.content = Some(trimmed);
        }
        _ => {}
    }
}

fn parse_sectioned_body(body: &str) -> ParsedSections {
    let mut sections = ParsedSections::default();
    let mut current = String::new();
    let mut current_name: Option<&'static str> = None;
    let mut saw_blank_after_structured_section = false;

    for line in body.lines() {
        let trimmed = line.trim();
        if current_name == Some("content") {
            current.push_str(line);
            current.push('\n');
            continue;
        }

        if current_name.is_none() {
            if let Some(title) = trimmed.strip_prefix("# ") {
                if sections.title.is_none() {
                    sections.title = Some(title.trim().to_string());
                }
                continue;
            }
        }

        let next_section = match trimmed {
            "## Summary" => Some("summary"),
            "## Maintenance Rules" => Some("rules"),
            "## Content" => Some("content"),
            _ => None,
        };

        if let Some(next_section) = next_section {
            if let Some(name) = current_name.take() {
                finish_section(&mut sections, name, &current);
            }
            current.clear();
            current_name = Some(next_section);
            saw_blank_after_structured_section = false;
            continue;
        }

        match current_name {
            Some("summary") | Some("rules") => {
                let starts_implicit_content = trimmed.starts_with("# ")
                    || (!trimmed.is_empty() && saw_blank_after_structured_section);
                if starts_implicit_content {
                    finish_section(
                        &mut sections,
                        current_name.take().expect("section name"),
                        &current,
                    );
                    current.clear();
                    current_name = Some("content");
                    saw_blank_after_structured_section = false;
                    current.push_str(line);
                    current.push('\n');
                    continue;
                }

                saw_blank_after_structured_section = trimmed.is_empty();
                current.push_str(line);
                current.push('\n');
            }
            None => {
                if trimmed.is_empty() {
                    continue;
                }
                current_name = Some("content");
                current.push_str(line);
                current.push('\n');
            }
            _ => unreachable!("content handled earlier"),
        }
    }

    if let Some(name) = current_name.take() {
        finish_section(&mut sections, name, &current);
    }

    sections
}

fn render_memory_comment_block(start: &str, end: &str, content: &str) -> String {
    let mut rendered = String::new();
    rendered.push_str(start);
    rendered.push('\n');
    let trimmed = content.trim_matches('\n');
    if !trimmed.is_empty() {
        rendered.push_str(trimmed);
        rendered.push('\n');
    }
    rendered.push_str(end);
    rendered
}

fn parse_memory_comment_sections(content: &str) -> Result<Option<MemoryCommentSections>, String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Block {
        MaintainRules,
        Body,
    }

    let normalized = normalize_markdown(content);
    let mut current: Option<Block> = None;
    let mut saw_marker = false;
    let mut saw_body_block = false;
    let mut rules = String::new();
    let mut body = String::new();

    for line in normalized.lines() {
        let trimmed = line.trim();
        match trimmed {
            MEMORY_MAINTAIN_RULES_START => {
                if current.is_some() {
                    return Err("Memory comment blocks cannot be nested".to_string());
                }
                saw_marker = true;
                current = Some(Block::MaintainRules);
                continue;
            }
            MEMORY_MAINTAIN_RULES_END => {
                if current != Some(Block::MaintainRules) {
                    return Err("Memory maintain-rules block is not closed correctly".to_string());
                }
                current = None;
                continue;
            }
            MEMORY_BODY_START => {
                if current.is_some() {
                    return Err("Memory comment blocks cannot be nested".to_string());
                }
                saw_marker = true;
                saw_body_block = true;
                current = Some(Block::Body);
                continue;
            }
            MEMORY_BODY_END => {
                if current != Some(Block::Body) {
                    return Err("Memory body block is not closed correctly".to_string());
                }
                current = None;
                continue;
            }
            _ => {}
        }

        match current {
            Some(Block::MaintainRules) => {
                rules.push_str(line);
                rules.push('\n');
            }
            Some(Block::Body) => {
                body.push_str(line);
                body.push('\n');
            }
            None => {}
        }
    }

    if current.is_some() {
        return Err("Memory comment block is not closed".to_string());
    }
    if !saw_marker {
        return Ok(None);
    }
    if !saw_body_block {
        return Err("Memory document missing body comment block".to_string());
    }

    let rules = rules.trim_matches('\n').to_string();
    let body = body.trim_matches('\n').to_string();

    Ok(Some(MemoryCommentSections {
        maintenance_rules: (!rules.trim().is_empty()).then_some(rules),
        body,
    }))
}

fn parse_memory_body_sections(
    sections: ParsedSections,
) -> Result<(Option<String>, Option<String>, String), String> {
    if let Some(content) = sections.content.as_deref() {
        if let Some(comment_sections) = parse_memory_comment_sections(content)? {
            return Ok((
                sections.summary,
                comment_sections.maintenance_rules,
                comment_sections.body,
            ));
        }
    }

    let body = sections
        .content
        .ok_or_else(|| "Knowledge document missing memory body".to_string())?;
    Ok((sections.summary, sections.maintenance_rules, body))
}

fn render_frontmatter(doc: &KnowledgeDocument) -> Result<String, String> {
    let frontmatter = KnowledgeFrontmatter {
        id: doc.id.clone(),
        doc_type: doc.doc_type,
        path: doc.path.clone(),
        title: doc.title.clone(),
        inject_mode: (!doc.inherit_inject_mode).then_some(doc.inject_mode),
        inherit_inject_mode: doc.inherit_inject_mode.then_some(true),
        summary_enabled: Some(doc.summary_enabled),
        summary_cache: (!doc.summary_enabled)
            .then(|| doc.summary.clone())
            .flatten(),
        command_enabled: doc.command_enabled,
        read_only: doc.read_only,
        ai_maintained: (!doc.inherit_ai_config).then_some(doc.ai_maintained),
        inherit_ai_config: doc.inherit_ai_config.then_some(true),
        explicit_maintenance_rules: (!doc.inherit_ai_config)
            .then_some(doc.explicit_maintenance_rules),
        maintenance_rules_cache: (!doc.inherit_ai_config && !doc.explicit_maintenance_rules)
            .then(|| doc.maintenance_rules.clone())
            .flatten(),
        external_source: doc.external_source.clone(),
        skill_enabled: doc.skill_enabled,
        skill_surface: doc.skill_surface,
        command_trigger: doc.command_trigger.clone(),
        argument_hint: doc.argument_hint.clone(),
        tools: doc.tools.clone(),
        created_at: doc.created_at,
        updated_at: doc.updated_at,
    };

    serde_yaml::to_string(&frontmatter)
        .map_err(|e| format!("Failed to serialize knowledge frontmatter: {}", e))
}

fn render_document_body(doc: &KnowledgeDocument) -> Result<String, String> {
    let mut rendered = String::new();
    rendered.push_str("# ");
    rendered.push_str(doc.title.trim());
    rendered.push_str("\n\n");

    if let Some(summary) = active_summary(doc) {
        rendered.push_str("## Summary\n");
        rendered.push_str(summary.trim());
        rendered.push_str("\n\n");
    }

    if doc.doc_type == KnowledgeType::Memory {
        if doc.explicit_maintenance_rules && !doc.inherit_ai_config {
            let rules = active_maintenance_rules(doc).unwrap_or_default();
            rendered.push_str(&render_memory_comment_block(
                MEMORY_MAINTAIN_RULES_START,
                MEMORY_MAINTAIN_RULES_END,
                rules,
            ));
            rendered.push_str("\n\n");
        } else if doc.ai_maintained && !doc.inherit_ai_config {
            return Err("aiMaintained=true requires maintenance rules".to_string());
        }

        rendered.push_str(&render_memory_comment_block(
            MEMORY_BODY_START,
            MEMORY_BODY_END,
            doc.body.trim_end(),
        ));
        rendered.push('\n');
        return Ok(rendered);
    }

    if doc.explicit_maintenance_rules && !doc.inherit_ai_config {
        rendered.push_str("## Maintenance Rules\n");
        if let Some(rules) = active_maintenance_rules(doc) {
            rendered.push_str(rules.trim());
            rendered.push('\n');
        }
        rendered.push('\n');
    } else if doc.ai_maintained && !doc.inherit_ai_config {
        return Err("aiMaintained=true requires maintenance rules".to_string());
    }

    rendered.push_str("## Content\n");
    rendered.push_str(doc.body.trim_end());
    rendered.push('\n');
    Ok(rendered)
}

fn render_document(doc: &KnowledgeDocument) -> Result<String, String> {
    let mut normalized = doc.clone();
    apply_external_source_defaults(&mut normalized);

    let mut rendered = String::new();
    rendered.push_str("---\n");
    rendered.push_str(&render_frontmatter(&normalized)?);
    rendered.push_str("---\n\n");
    rendered.push_str(&render_document_body(&normalized)?);
    Ok(rendered)
}

pub fn rendered_document_size_bytes(doc: &KnowledgeDocument) -> Result<u64, String> {
    Ok(render_document(doc)?.len() as u64)
}

pub fn render_document_preview(doc: &KnowledgeDocument) -> Result<String, String> {
    if doc.inherit_ai_config && doc.explicit_maintenance_rules {
        let mut preview = doc.clone();
        preview.inherit_ai_config = false;
        return render_document_body(&preview);
    }
    render_document_body(doc)
}

fn validate_document(doc: &KnowledgeDocument) -> Result<(), String> {
    if doc.id.trim().is_empty() {
        return Err("Knowledge document id is required".to_string());
    }
    if doc.path.trim().is_empty() {
        return Err("Knowledge document path is required".to_string());
    }
    if doc.title.trim().is_empty() {
        return Err("Knowledge document title is required".to_string());
    }
    if doc.ai_maintained && !doc.explicit_maintenance_rules {
        return Err("aiMaintained=true requires explicitMaintenanceRules=true".to_string());
    }
    if doc.ai_maintained
        && doc
            .maintenance_rules
            .as_ref()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
    {
        return Err("aiMaintained=true requires non-empty maintenance rules".to_string());
    }
    if matches!(
        doc.doc_type,
        KnowledgeType::Skill | KnowledgeType::Reference
    ) && matches!(
        doc.inject_mode,
        KnowledgeInjectMode::Full | KnowledgeInjectMode::Rule
    ) {
        return Err(
            "skill/reference documents cannot use injectMode=full or injectMode=rule".to_string(),
        );
    }
    if doc.doc_type == KnowledgeType::Skill {
        if doc.skill_enabled.is_none() || doc.skill_surface.is_none() {
            return Err("skill documents require skillEnabled and skillSurface".to_string());
        }
    }
    Ok(())
}

fn parse_document(content: &str, path_hint: Option<&str>) -> Result<KnowledgeDocument, String> {
    let (frontmatter, body) = parse_frontmatter(content)?;
    let sections = parse_sectioned_body(body);
    let path = if frontmatter.path.trim().is_empty() {
        normalize_relative_path(path_hint.unwrap_or(&frontmatter.path))?
    } else {
        normalize_relative_path(&frontmatter.path)?
    };
    let title = default_document_title_from_path(&path)?;
    let (summary, parsed_rules, body) = if frontmatter.doc_type == KnowledgeType::Memory {
        parse_memory_body_sections(sections)?
    } else {
        (
            sections.summary,
            sections.maintenance_rules,
            sections
                .content
                .ok_or_else(|| "Knowledge document missing ## Content section".to_string())?,
        )
    };
    let summary = summary.or(frontmatter.summary_cache.clone());
    let inherit_inject_mode = frontmatter.inherit_inject_mode.unwrap_or(false);
    let inherit_ai_config = frontmatter.inherit_ai_config.unwrap_or(false);
    let has_rules = has_maintenance_rules_content(parsed_rules.as_deref());
    let maintenance_rules = if inherit_ai_config {
        None
    } else {
        parsed_rules.or(frontmatter.maintenance_rules_cache.clone())
    };
    let summary_enabled = frontmatter.summary_enabled.unwrap_or_else(|| {
        has_summary_content(summary.as_deref())
            || default_summary_enabled_for_type(frontmatter.doc_type)
    });

    let mut doc = KnowledgeDocument {
        id: frontmatter.id,
        doc_type: frontmatter.doc_type,
        path,
        title,
        inject_mode: frontmatter
            .inject_mode
            .unwrap_or_else(|| default_document_inject_mode_for_type(frontmatter.doc_type)),
        inherit_inject_mode,
        inject_mode_source: if inherit_inject_mode {
            type_default_config_source()
        } else {
            self_config_source()
        },
        summary_enabled,
        command_enabled: frontmatter.command_enabled,
        read_only: frontmatter.read_only,
        ai_maintained: frontmatter
            .ai_maintained
            .unwrap_or_else(|| default_ai_maintained_for_type(frontmatter.doc_type)),
        storage_source: KnowledgeStorageSource::Project,
        inherit_ai_config,
        ai_config_source: if inherit_ai_config {
            type_default_config_source()
        } else {
            self_config_source()
        },
        explicit_maintenance_rules: frontmatter.explicit_maintenance_rules.unwrap_or_else(|| {
            default_explicit_maintenance_rules_for_type(frontmatter.doc_type)
                || frontmatter.ai_maintained.unwrap_or(false)
                || has_rules
        }),
        external_source: frontmatter.external_source,
        skill_enabled: frontmatter.skill_enabled,
        skill_surface: frontmatter.skill_surface,
        command_trigger: frontmatter.command_trigger,
        argument_hint: frontmatter.argument_hint,
        tools: normalize_tool_names(frontmatter.tools),
        summary,
        body,
        maintenance_rules,
        created_at: frontmatter.created_at,
        updated_at: frontmatter.updated_at,
    };

    if doc.created_at == 0 {
        doc.created_at = now_millis();
    }
    if doc.updated_at == 0 {
        doc.updated_at = doc.created_at;
    }
    apply_external_source_defaults(&mut doc);
    resolve_document_inheritance(None, &mut doc)?;
    ensure_summary_state(&mut doc);
    ensure_maintenance_rules(&mut doc);
    ensure_skill_defaults(&mut doc);
    apply_read_only_policy(&mut doc);
    validate_document(&doc)?;

    if let Some(hint) = path_hint {
        let normalized = normalize_relative_path(hint)?;
        if doc.path != normalized {
            return Err(format!(
                "Knowledge document path '{}' does not match the file path '{}'",
                doc.path, normalized
            ));
        }
    }

    Ok(doc)
}

pub fn ensure_document_path(path: &str) -> Result<String, String> {
    normalize_relative_path(path)
}

pub fn ensure_directory_path(path: &str) -> Result<String, String> {
    normalize_relative_directory_path(path)
}

pub fn move_directory(
    working_dir: &str,
    doc_type: KnowledgeType,
    source_path: &str,
    target_path: &str,
) -> Result<String, String> {
    ensure_knowledge_roots(working_dir)?;

    let source_rel = normalize_relative_directory_path(source_path)?;
    let target_rel = normalize_relative_directory_path(target_path)?;
    if source_rel == target_rel {
        return Ok(target_rel);
    }
    if target_rel.starts_with(&format!("{source_rel}/")) {
        return Err("Knowledge directory cannot be moved into its own descendant".to_string());
    }

    let type_root = type_root(working_dir, doc_type);
    let source_dir = type_root.join(&source_rel);
    if !source_dir.is_dir() {
        return Err(format!("Knowledge directory not found: {}", source_rel));
    }
    let (source_config_path, _) = directory_config_path_in_type_root(&type_root, &source_rel)?;

    let target_dir = type_root.join(&target_rel);
    if target_dir.exists() {
        return Err(format!(
            "Knowledge directory already exists: {}",
            target_rel
        ));
    }
    let (target_config_path, _) = directory_config_path_in_type_root(&type_root, &target_rel)?;
    if target_config_path.exists() {
        return Err(format!(
            "Knowledge directory config already exists: {}",
            target_config_path.display()
        ));
    }

    let mut directories = Vec::new();
    let mut documents = Vec::new();
    let mut sidecar_files = Vec::new();

    for entry in WalkDir::new(&source_dir).min_depth(0).into_iter().flatten() {
        let relative = entry
            .path()
            .strip_prefix(&source_dir)
            .map_err(|e| format!("Failed to resolve directory move path: {}", e))?;
        let relative_str = relative.to_string_lossy().replace('\\', "/");

        if entry.file_type().is_dir() {
            directories.push(relative.to_path_buf());
            continue;
        }

        let is_markdown = entry
            .path()
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false);
        if !is_markdown && !is_directory_config_file(entry.path()) {
            return Err(format!(
                "Knowledge directory move only supports Markdown files and directory config sidecars: {}",
                entry.path().display()
            ));
        }
        if is_directory_config_file(entry.path()) {
            sidecar_files.push(relative.to_path_buf());
            continue;
        }

        let old_doc_path = if relative_str.is_empty() {
            source_rel.clone()
        } else {
            format!("{}/{}", source_rel, relative_str)
        };
        let new_doc_path = if relative_str.is_empty() {
            target_rel.clone()
        } else {
            format!("{}/{}", target_rel, relative_str)
        };

        let doc = load_document_by_path(working_dir, doc_type, &old_doc_path)?;
        if doc.read_only {
            return Err(format!(
                "Cannot move knowledge directory because '{}' is read-only",
                old_doc_path
            ));
        }

        documents.push((old_doc_path, new_doc_path));
    }

    for relative_dir in &directories {
        std::fs::create_dir_all(target_dir.join(relative_dir)).map_err(|e| {
            format!(
                "Failed to create target knowledge directory '{}': {}",
                target_dir.join(relative_dir).display(),
                e
            )
        })?;
    }

    for (old_doc_path, new_doc_path) in documents {
        update_document(
            working_dir,
            KnowledgeUpdateRequest {
                op: KnowledgeUpdateOp::UpdateMeta,
                path: old_doc_path,
                doc_type: Some(doc_type),
                new_path: Some(new_doc_path),
                ..Default::default()
            },
        )?;
    }

    if source_config_path.is_file() {
        if let Some(parent) = target_config_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Failed to create knowledge directory config parent '{}': {}",
                    parent.display(),
                    e
                )
            })?;
        }
        std::fs::rename(&source_config_path, &target_config_path).map_err(|e| {
            format!(
                "Failed to move knowledge directory config '{}' -> '{}': {}",
                source_config_path.display(),
                target_config_path.display(),
                e
            )
        })?;
    }

    for relative_sidecar in sidecar_files {
        let source_sidecar = source_dir.join(&relative_sidecar);
        let target_sidecar = target_dir.join(&relative_sidecar);
        if let Some(parent) = target_sidecar.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Failed to create knowledge directory sidecar parent '{}': {}",
                    parent.display(),
                    e
                )
            })?;
        }
        std::fs::rename(&source_sidecar, &target_sidecar).map_err(|e| {
            format!(
                "Failed to move knowledge directory sidecar '{}' -> '{}': {}",
                source_sidecar.display(),
                target_sidecar.display(),
                e
            )
        })?;
    }

    let mut cleanup_dirs = directories
        .into_iter()
        .map(|relative| source_dir.join(relative))
        .collect::<Vec<_>>();
    cleanup_dirs.sort_by(|left, right| {
        let left_depth = left.components().count();
        let right_depth = right.components().count();
        right_depth.cmp(&left_depth)
    });

    for dir in cleanup_dirs {
        if !dir.is_dir() {
            continue;
        }
        let is_empty = std::fs::read_dir(&dir)
            .map_err(|e| {
                format!(
                    "Failed to inspect knowledge directory '{}': {}",
                    dir.display(),
                    e
                )
            })?
            .next()
            .is_none();
        if is_empty {
            std::fs::remove_dir(&dir).map_err(|e| {
                format!(
                    "Failed to delete knowledge directory '{}': {}",
                    dir.display(),
                    e
                )
            })?;
        }
    }

    Ok(target_rel)
}

pub fn delete_directory(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<String, String> {
    delete_directory_internal(working_dir, doc_type, path, false)
}

fn load_document_for_directory_delete(
    working_dir: &str,
    doc_type: KnowledgeType,
    rel_path: &str,
) -> Result<KnowledgeDocument, String> {
    match load_document_by_path(working_dir, doc_type, rel_path) {
        Ok(document) => Ok(document),
        Err(load_error) => {
            let file_path = document_path(working_dir, doc_type, rel_path)?;
            let raw = read_raw_document(&file_path)?;
            let mut document = parse_document(&raw, None).map_err(|parse_error| {
                format!(
                    "Failed to inspect knowledge document '{}' for directory delete: {}. Fallback parse failed: {}",
                    rel_path, load_error, parse_error
                )
            })?;
            resolve_document_inheritance(Some(working_dir), &mut document)?;
            ensure_maintenance_rules(&mut document);
            validate_document(&document)?;
            Ok(document)
        }
    }
}

fn delete_directory_internal(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
    allow_read_only_documents: bool,
) -> Result<String, String> {
    ensure_knowledge_roots(working_dir)?;

    let target_rel = normalize_relative_directory_path(path)?;
    let type_root = type_root(working_dir, doc_type);
    let target_dir = type_root.join(&target_rel);
    if !target_dir.is_dir() {
        return Err(format!("Knowledge directory not found: {}", target_rel));
    }
    let (target_config_path, _) = directory_config_path_in_type_root(&type_root, &target_rel)?;

    let canonical_root = dunce::canonicalize(&type_root)
        .map_err(|e| format!("Failed to resolve knowledge root: {}", e))?;
    let canonical_target = dunce::canonicalize(&target_dir)
        .map_err(|e| format!("Failed to resolve knowledge directory: {}", e))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err("Knowledge directory resolves outside of the knowledge root".to_string());
    }

    for entry in WalkDir::new(&target_dir).min_depth(1).into_iter().flatten() {
        if entry.file_type().is_dir() {
            continue;
        }

        let is_markdown = entry
            .path()
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false);
        if !is_markdown && !is_directory_config_file(entry.path()) {
            return Err(format!(
                "Knowledge directory delete only supports Markdown files and directory config sidecars: {}",
                entry.path().display()
            ));
        }
        if is_directory_config_file(entry.path()) {
            continue;
        }

        let doc_path = entry
            .path()
            .strip_prefix(&type_root)
            .map_err(|e| format!("Failed to resolve directory delete path: {}", e))?
            .to_string_lossy()
            .replace('\\', "/");
        let doc = load_document_for_directory_delete(working_dir, doc_type, &doc_path)?;
        if doc.read_only && !allow_read_only_documents {
            return Err(format!(
                "Cannot delete knowledge directory because '{}' is read-only",
                doc_path
            ));
        }
    }

    std::fs::remove_dir_all(&target_dir).map_err(|e| {
        format!(
            "Failed to delete knowledge directory '{}': {}",
            target_dir.display(),
            e
        )
    })?;

    if target_config_path.is_file() {
        std::fs::remove_file(&target_config_path).map_err(|e| {
            format!(
                "Failed to delete knowledge directory config '{}': {}",
                target_config_path.display(),
                e
            )
        })?;
    }

    Ok(target_rel)
}

pub fn delete_directory_config_sidecars(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<(), String> {
    let dir_path = normalize_relative_directory_path(path)?;
    let type_root = type_root(working_dir, doc_type);
    let (current_config_path, _) = directory_config_path_in_type_root(&type_root, &dir_path)?;
    let (legacy_config_path, _) = legacy_directory_config_path_in_type_root(&type_root, &dir_path)?;

    for (config_path, label) in [
        (current_config_path, "knowledge directory config"),
        (legacy_config_path, "legacy knowledge directory config"),
    ] {
        match std::fs::remove_file(&config_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Failed to delete {} '{}': {}",
                    label,
                    config_path.display(),
                    error
                ));
            }
        }
    }

    Ok(())
}

pub fn delete_external_reference_directory(
    working_dir: &str,
    path: &str,
) -> Result<String, String> {
    let target_rel = normalize_relative_directory_path(path)?;
    let record = read_directory_config(working_dir, KnowledgeType::Reference, &target_rel)?;
    if record.external_sources.is_empty() {
        return Err("Reference directory is not bound to an external source".to_string());
    }
    if record.external_sources.iter().any(|source| {
        matches!(
            source.provider,
            KnowledgeSourceProvider::Feishu | KnowledgeSourceProvider::Unity
        )
    }) {
        return Err(
            "Managed reference provider directories must use the provider-specific delete flow"
                .to_string(),
        );
    }
    delete_directory_internal(working_dir, KnowledgeType::Reference, &target_rel, true)
}

pub fn read_document_from_file(path: &Path) -> Result<KnowledgeDocument, String> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "Failed to read knowledge document '{}': {}",
            path.display(),
            e
        )
    })?;
    let rel = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    parse_document(&content, Some(&rel))
}

pub fn load_document_by_path(
    working_dir: &str,
    doc_type: KnowledgeType,
    rel_path: &str,
) -> Result<KnowledgeDocument, String> {
    let normalized_path = normalize_relative_path(rel_path)?;
    if doc_type == KnowledgeType::Reference {
        if let Some(document) = unity_docs::load_managed_document(working_dir, &normalized_path)? {
            return Ok(document);
        }
    }
    let path = document_path(working_dir, doc_type, &normalized_path)?;
    if !path.is_file() {
        return Err(format!(
            "Knowledge document not found: {}/{}",
            doc_type.as_str(),
            normalized_path
        ));
    }
    let content = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "Failed to read knowledge document '{}': {}",
            path.display(),
            e
        )
    })?;
    let mut document = parse_document(&content, Some(&normalized_path))?;
    resolve_document_inheritance(Some(working_dir), &mut document)?;
    ensure_maintenance_rules(&mut document);
    validate_document(&document)?;
    Ok(document)
}

pub fn load_document_by_root(
    knowledge_root: &Path,
    doc_type: KnowledgeType,
    rel_path: &str,
) -> Result<KnowledgeDocument, String> {
    let path = document_path_in_root(knowledge_root, doc_type, rel_path)?;
    if !path.is_file() {
        return Err(format!(
            "Knowledge document not found: {}/{}",
            doc_type.as_str(),
            normalize_relative_path(rel_path)?
        ));
    }
    let content = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "Failed to read knowledge document '{}': {}",
            path.display(),
            e
        )
    })?;
    let mut document = parse_document(&content, Some(rel_path))?;
    resolve_document_inheritance_from_root(Some(knowledge_root), &mut document)?;
    ensure_maintenance_rules(&mut document);
    validate_document(&document)?;
    Ok(document)
}

fn overlay_app_document_metadata(document: &mut KnowledgeDocument) {
    document.storage_source = KnowledgeStorageSource::App;
    document.read_only = true;
}

fn overlay_app_directory_read_only(record: &mut KnowledgeDirectoryConfigRecord) {
    record.read_only = true;
}

fn knowledge_item_key(doc_type: KnowledgeType, path: &str) -> String {
    format!("{}/{}", doc_type.as_str(), path)
}

fn should_parallelize_document_load(count: usize) -> bool {
    count >= DOCUMENT_LOAD_PARALLEL_THRESHOLD
        && std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1)
            > 1
}

fn load_documents_from_root_paths(
    knowledge_root: &Path,
    doc_type: KnowledgeType,
    relative_paths: &[String],
) -> Vec<KnowledgeDocument> {
    let load_document = |relative_path: &String| {
        load_document_by_root(knowledge_root, doc_type, relative_path).ok()
    };

    if should_parallelize_document_load(relative_paths.len()) {
        relative_paths
            .par_iter()
            .filter_map(load_document)
            .collect()
    } else {
        relative_paths.iter().filter_map(load_document).collect()
    }
}

fn document_to_list_item(doc: KnowledgeDocument) -> KnowledgeListItem {
    let has_summary = active_summary(&doc).is_some();
    let has_body_content_flag = has_body_content(&doc.body);
    let byte_size = rendered_document_size_bytes(&doc).ok();
    KnowledgeListItem {
        id: doc.id,
        doc_type: doc.doc_type,
        path: doc.path,
        title: doc.title,
        inject_mode: doc.inject_mode,
        summary_enabled: doc.summary_enabled,
        command_enabled: doc.command_enabled,
        read_only: doc.read_only,
        ai_maintained: doc.ai_maintained,
        explicit_maintenance_rules: doc.explicit_maintenance_rules,
        storage_source: doc.storage_source,
        external_source: doc.external_source,
        skill_enabled: doc.skill_enabled,
        skill_surface: doc.skill_surface,
        command_trigger: doc.command_trigger,
        argument_hint: doc.argument_hint,
        created_at: doc.created_at,
        updated_at: doc.updated_at,
        has_summary,
        has_body_content: has_body_content_flag,
        byte_size,
        lexical_search_enabled: None,
        semantic_search_enabled: None,
        summary: doc.summary,
    }
}

fn collect_document_snapshots_from_root(
    knowledge_root: &Path,
    doc_type: KnowledgeType,
    normalized_prefix: Option<&str>,
    excluded_prefixes: Option<&[String]>,
    app_read_only: bool,
    storage_source: KnowledgeStorageSource,
    seen: &mut std::collections::HashSet<String>,
    items: &mut Vec<KnowledgeDocument>,
) -> Result<(), String> {
    let root = type_root_in_knowledge_root(knowledge_root, doc_type);
    if !root.is_dir() {
        return Ok(());
    }

    let relative_paths = WalkDir::new(&root)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                return None;
            }
            let relative_path = path
                .strip_prefix(&root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            if excluded_prefixes
                .map(|prefixes| path_matches_any_prefix(&relative_path, prefixes))
                .unwrap_or(false)
            {
                return None;
            }
            Some(relative_path)
        })
        .collect::<Vec<_>>();

    for mut doc in load_documents_from_root_paths(knowledge_root, doc_type, &relative_paths) {
        if let Some(prefix) = normalized_prefix {
            if !doc.path.starts_with(prefix) {
                continue;
            }
        }
        let key = knowledge_item_key(doc.doc_type, &doc.path);
        if !seen.insert(key) {
            continue;
        }
        if storage_source == KnowledgeStorageSource::App {
            overlay_app_document_metadata(&mut doc);
        } else if app_read_only {
            doc.read_only = true;
        }
        items.push(doc);
    }

    Ok(())
}

fn normalize_excluded_prefixes(
    excluded_prefixes: &[(KnowledgeType, String)],
) -> Result<std::collections::HashMap<KnowledgeType, Vec<String>>, String> {
    let mut grouped = std::collections::HashMap::<KnowledgeType, Vec<String>>::new();
    for (doc_type, prefix) in excluded_prefixes {
        grouped
            .entry(*doc_type)
            .or_default()
            .push(normalize_relative_prefix(prefix)?);
    }
    Ok(grouped)
}

fn path_is_excluded(
    doc_type: KnowledgeType,
    relative_path: &str,
    excluded_prefixes: &std::collections::HashMap<KnowledgeType, Vec<String>>,
) -> bool {
    excluded_prefixes
        .get(&doc_type)
        .map(|prefixes| path_matches_any_prefix(relative_path, prefixes))
        .unwrap_or(false)
}

fn managed_reference_listing_fully_excluded(
    path_prefix: Option<&str>,
    excluded_prefixes: &std::collections::HashMap<KnowledgeType, Vec<String>>,
) -> bool {
    let Some(reference_exclusions) = excluded_prefixes.get(&KnowledgeType::Reference) else {
        return false;
    };
    let target_prefix = path_prefix
        .filter(|value| crate::unity_docs::is_unity_reference_managed_relative_path(value))
        .unwrap_or(crate::unity_docs::UNITY_REFERENCE_MANAGED_DIR);
    path_matches_any_prefix(target_prefix, reference_exclusions)
}

fn path_matches_any_prefix(path: &str, prefixes: &[String]) -> bool {
    prefixes
        .iter()
        .any(|prefix| path_matches_prefix(path, prefix))
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .map(|suffix| suffix.starts_with('/'))
            .unwrap_or(false)
}

fn should_skip_workspace_managed_reference_dir(
    working_dir: &str,
    doc_type: KnowledgeType,
    type_root: &Path,
    candidate_path: &Path,
) -> bool {
    if doc_type != KnowledgeType::Reference || !crate::unity_docs::has_managed_store(working_dir) {
        return false;
    }
    let Ok(relative) = candidate_path.strip_prefix(type_root) else {
        return false;
    };
    let relative = relative.to_string_lossy().replace('\\', "/");
    crate::unity_docs::is_unity_reference_managed_relative_path(&relative)
}

fn collect_directories_from_root(
    knowledge_root: &Path,
    doc_type: KnowledgeType,
    directories: &mut std::collections::BTreeSet<String>,
) {
    let type_root = type_root_in_knowledge_root(knowledge_root, doc_type);
    if !type_root.is_dir() {
        return;
    }

    for entry in WalkDir::new(&type_root).min_depth(1).into_iter().flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(&type_root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.is_empty() {
            continue;
        }
        directories.insert(relative);
    }
}

pub fn load_document_by_id(
    working_dir: &str,
    id: &str,
) -> Result<Option<KnowledgeDocument>, String> {
    for doc_type in KnowledgeType::all() {
        let items = list_documents(working_dir, Some(doc_type), None)?;
        for item in items {
            if item.id == id {
                return load_document_by_path(working_dir, doc_type, &item.path).map(Some);
            }
        }
    }
    Ok(None)
}

pub fn load_document_by_path_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: KnowledgeType,
    rel_path: &str,
) -> Result<KnowledgeDocument, String> {
    let normalized_path = normalize_relative_path(rel_path)?;
    if doc_type == KnowledgeType::Reference {
        if let Some(document) = unity_docs::load_managed_document(working_dir, &normalized_path)? {
            return Ok(document);
        }
    }
    let workspace_path = document_path(working_dir, doc_type, &normalized_path)?;
    if workspace_path.is_file() {
        return load_document_by_path(working_dir, doc_type, &normalized_path);
    }

    if let Some(app_root) = app_knowledge_dir {
        let app_path = document_path_in_root(app_root, doc_type, &normalized_path)?;
        if app_path.is_file() {
            let mut document = load_document_by_root(app_root, doc_type, &normalized_path)?;
            overlay_app_document_metadata(&mut document);
            return Ok(document);
        }
    }

    Err(format!(
        "Knowledge document not found: {}/{}",
        doc_type.as_str(),
        normalized_path
    ))
}

pub fn list_documents_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: Option<KnowledgeType>,
    path_prefix: Option<&str>,
) -> Result<Vec<KnowledgeListItem>, String> {
    Ok(
        load_documents_with_app_root(working_dir, app_knowledge_dir, doc_type, path_prefix)?
            .into_iter()
            .map(document_to_list_item)
            .collect(),
    )
}

pub fn list_documents_with_app_root_excluding_prefixes(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: Option<KnowledgeType>,
    path_prefix: Option<&str>,
    excluded_prefixes: &[(KnowledgeType, String)],
) -> Result<Vec<KnowledgeListItem>, String> {
    Ok(load_documents_with_app_root_excluding_prefixes(
        working_dir,
        app_knowledge_dir,
        doc_type,
        path_prefix,
        excluded_prefixes,
    )?
    .into_iter()
    .map(document_to_list_item)
    .collect())
}

pub fn load_documents_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: Option<KnowledgeType>,
    path_prefix: Option<&str>,
) -> Result<Vec<KnowledgeDocument>, String> {
    load_documents_with_app_root_excluding_prefixes(
        working_dir,
        app_knowledge_dir,
        doc_type,
        path_prefix,
        &[],
    )
}

pub fn load_documents_with_app_root_excluding_prefixes(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: Option<KnowledgeType>,
    path_prefix: Option<&str>,
    excluded_prefixes: &[(KnowledgeType, String)],
) -> Result<Vec<KnowledgeDocument>, String> {
    ensure_knowledge_roots(working_dir)?;
    ensure_memory_builtin_documents(working_dir)?;

    let mut documents = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let types: Vec<_> = doc_type
        .map(|value| vec![value])
        .unwrap_or_else(|| KnowledgeType::all().to_vec());
    if types.iter().any(|value| *value == KnowledgeType::Reference) {
        unity_docs::ensure_managed_store_available(working_dir)?;
    }
    let normalized_prefix = path_prefix.map(normalize_relative_prefix).transpose()?;
    let normalized_exclusions = normalize_excluded_prefixes(excluded_prefixes)?;
    let workspace_knowledge_root = knowledge_root(working_dir);

    for ty in types {
        let root = type_root(working_dir, ty);
        if root.is_dir() {
            let relative_paths = WalkDir::new(&root)
                .into_iter()
                .filter_entry(|entry| {
                    !should_skip_workspace_managed_reference_dir(
                        working_dir,
                        ty,
                        &root,
                        entry.path(),
                    )
                })
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let path = entry.path();
                    if !path.is_file()
                        || path.extension().and_then(|ext| ext.to_str()) != Some("md")
                    {
                        return None;
                    }
                    let relative_path = path
                        .strip_prefix(&root)
                        .ok()?
                        .to_string_lossy()
                        .replace('\\', "/");
                    if ty == KnowledgeType::Reference
                        && unity_docs::has_managed_store(working_dir)
                        && unity_docs::is_unity_reference_managed_relative_path(&relative_path)
                    {
                        return None;
                    }
                    if path_is_excluded(ty, &relative_path, &normalized_exclusions) {
                        return None;
                    }
                    Some(relative_path)
                })
                .collect::<Vec<_>>();

            for doc in
                load_documents_from_root_paths(&workspace_knowledge_root, ty, &relative_paths)
            {
                if let Some(prefix) = normalized_prefix.as_deref() {
                    if !doc.path.starts_with(prefix) {
                        continue;
                    }
                }
                let key = knowledge_item_key(doc.doc_type, &doc.path);
                if !seen.insert(key) {
                    continue;
                }
                documents.push(doc);
            }
        }

        if ty == KnowledgeType::Reference
            && !managed_reference_listing_fully_excluded(
                normalized_prefix.as_deref(),
                &normalized_exclusions,
            )
        {
            for doc in
                unity_docs::list_managed_documents(working_dir, normalized_prefix.as_deref())?
            {
                if path_is_excluded(ty, &doc.path, &normalized_exclusions) {
                    continue;
                }
                let key = knowledge_item_key(doc.doc_type, &doc.path);
                if !seen.insert(key) {
                    continue;
                }
                documents.push(doc);
            }
        }

        if let Some(app_root) = app_knowledge_dir {
            collect_document_snapshots_from_root(
                app_root,
                ty,
                normalized_prefix.as_deref(),
                normalized_exclusions
                    .get(&ty)
                    .map(|prefixes| prefixes.as_slice()),
                true,
                KnowledgeStorageSource::App,
                &mut seen,
                &mut documents,
            )?;
        }
    }

    documents.sort_by(|a, b| {
        a.doc_type
            .as_str()
            .cmp(b.doc_type.as_str())
            .then(a.path.cmp(&b.path))
            .then(a.title.cmp(&b.title))
    });
    Ok(documents)
}

pub fn read_document_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: KnowledgeType,
    path: &str,
    part: &str,
) -> Result<KnowledgeReadResult, String> {
    let normalized_path = normalize_relative_path(path)?;
    let document = load_document_by_path_with_app_root(
        working_dir,
        app_knowledge_dir,
        doc_type,
        &normalized_path,
    )?;
    let file_path = resolve_document_file_path_with_app_root(
        working_dir,
        app_knowledge_dir,
        doc_type,
        &normalized_path,
    )
    .ok();
    let file_metadata = file_path
        .as_deref()
        .map(|value| build_document_file_metadata(Some(working_dir), value, &document));
    build_read_result(document, part, file_metadata)
}

pub fn read_directory_config_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<KnowledgeDirectoryConfigRecord, String> {
    let normalized_path = normalize_relative_directory_path(path)?;
    if directory_exists(working_dir, doc_type, &normalized_path)? {
        return read_directory_config(working_dir, doc_type, &normalized_path);
    }

    if let Some(app_root) = app_knowledge_dir {
        let app_dir = type_root_in_knowledge_root(app_root, doc_type).join(&normalized_path);
        if app_dir.is_dir() {
            let mut record = read_directory_config_from_root(app_root, doc_type, &normalized_path)?;
            overlay_app_directory_read_only(&mut record);
            return Ok(record);
        }
    }

    Err(format!(
        "Knowledge directory not found: {}",
        normalized_path
    ))
}

pub fn list_documents(
    working_dir: &str,
    doc_type: Option<KnowledgeType>,
    path_prefix: Option<&str>,
) -> Result<Vec<KnowledgeListItem>, String> {
    ensure_knowledge_roots(working_dir)?;
    let mut items = Vec::new();
    let types: Vec<_> = doc_type
        .map(|value| vec![value])
        .unwrap_or_else(|| KnowledgeType::all().to_vec());
    if types.iter().any(|value| *value == KnowledgeType::Reference) {
        unity_docs::ensure_managed_store_available(working_dir)?;
    }
    let normalized_prefix = path_prefix.map(normalize_relative_prefix).transpose()?;

    for ty in types {
        let root = type_root(working_dir, ty);
        if !root.is_dir() {
            continue;
        }

        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_entry(|entry| {
                !should_skip_workspace_managed_reference_dir(working_dir, ty, &root, entry.path())
            })
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            let Ok(relative_path) = path.strip_prefix(&root) else {
                continue;
            };
            let relative_path = relative_path.to_string_lossy().replace('\\', "/");
            if ty == KnowledgeType::Reference
                && unity_docs::has_managed_store(working_dir)
                && unity_docs::is_unity_reference_managed_relative_path(&relative_path)
            {
                continue;
            }
            let Ok(doc) = load_document_by_path(working_dir, ty, &relative_path) else {
                continue;
            };
            if let Some(prefix) = normalized_prefix.as_ref() {
                if !doc.path.starts_with(prefix) {
                    continue;
                }
            }
            items.push(document_to_list_item(doc));
        }

        if ty == KnowledgeType::Reference {
            for doc in
                unity_docs::list_managed_documents(working_dir, normalized_prefix.as_deref())?
            {
                items.push(document_to_list_item(doc));
            }
        }
    }

    items.sort_by(|a, b| {
        a.doc_type
            .as_str()
            .cmp(b.doc_type.as_str())
            .then(a.path.cmp(&b.path))
            .then(a.title.cmp(&b.title))
    });
    Ok(items)
}

pub fn list_directories(working_dir: &str, doc_type: KnowledgeType) -> Result<Vec<String>, String> {
    ensure_knowledge_roots(working_dir)?;
    if doc_type == KnowledgeType::Reference {
        crate::unity_docs::ensure_managed_store_available(working_dir)?;
    }

    let type_root = knowledge_root(working_dir).join(doc_type.as_str());
    let mut directories = Vec::new();
    if type_root.is_dir() {
        for entry in WalkDir::new(&type_root)
            .min_depth(1)
            .into_iter()
            .filter_entry(|entry| {
                !should_skip_workspace_managed_reference_dir(
                    working_dir,
                    doc_type,
                    &type_root,
                    entry.path(),
                )
            })
            .flatten()
        {
            if !entry.file_type().is_dir() {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(&type_root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if relative.is_empty() {
                continue;
            }
            directories.push(relative);
        }
    }
    if doc_type == KnowledgeType::Reference {
        directories.extend(crate::unity_docs::list_managed_directories(working_dir)?);
    }

    directories.sort();
    directories.dedup();
    Ok(directories)
}

pub fn list_directories_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: KnowledgeType,
) -> Result<Vec<String>, String> {
    ensure_knowledge_roots(working_dir)?;

    let mut directories = std::collections::BTreeSet::new();
    for directory in list_directories(working_dir, doc_type)? {
        directories.insert(directory);
    }
    if let Some(app_root) = app_knowledge_dir {
        collect_directories_from_root(app_root, doc_type, &mut directories);
    }

    Ok(directories.into_iter().collect())
}

pub fn list_directory_configs(
    working_dir: &str,
    doc_type: KnowledgeType,
) -> Result<Vec<KnowledgeDirectoryConfigRecord>, String> {
    let directories = list_directories(working_dir, doc_type)?;
    let mut records = Vec::with_capacity(directories.len());
    for directory in directories {
        records.push(read_directory_config(working_dir, doc_type, &directory)?);
    }
    Ok(records)
}

pub fn list_reference_external_directory_bindings(
    working_dir: &str,
) -> Result<Vec<KnowledgeExternalDirectoryBinding>, String> {
    ensure_knowledge_roots(working_dir)?;

    let reference_root = type_root(working_dir, KnowledgeType::Reference);
    if !reference_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut bindings =
        std::collections::BTreeMap::<String, KnowledgeExternalDirectoryBinding>::new();
    for entry in WalkDir::new(&reference_root)
        .min_depth(1)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() || !is_directory_config_file(entry.path()) {
            continue;
        }

        let stored = read_stored_directory_config(entry.path())?;
        if stored.external_sources.is_empty() {
            continue;
        }

        let path = directory_path_from_config_file(&reference_root, entry.path())?;
        let binding = KnowledgeExternalDirectoryBinding {
            path: path.clone(),
            external_sources: stored.external_sources,
        };
        let file_name = entry.file_name().to_string_lossy();
        let prefers_current_suffix = file_name.ends_with(KNOWLEDGE_DIRECTORY_CONFIG_SUFFIX);
        match bindings.get(&path) {
            Some(existing) if !prefers_current_suffix && !existing.external_sources.is_empty() => {
                continue;
            }
            _ => {
                bindings.insert(path, binding);
            }
        }
    }

    Ok(bindings.into_values().collect())
}

pub fn find_reference_directory_by_external_provider(
    working_dir: &str,
    provider: KnowledgeSourceProvider,
) -> Result<Option<KnowledgeDirectoryConfigRecord>, String> {
    let directories = list_directory_configs(working_dir, KnowledgeType::Reference)?;
    Ok(directories.into_iter().find(|record| {
        record
            .external_sources
            .iter()
            .any(|source| source.provider == provider)
    }))
}

pub fn list_directory_configs_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: KnowledgeType,
) -> Result<Vec<KnowledgeDirectoryConfigRecord>, String> {
    let directories = list_directories_with_app_root(working_dir, app_knowledge_dir, doc_type)?;
    let mut records = Vec::with_capacity(directories.len());
    for directory in directories {
        records.push(read_directory_config_with_app_root(
            working_dir,
            app_knowledge_dir,
            doc_type,
            &directory,
        )?);
    }
    Ok(records)
}

pub fn list_directory_configs_with_app_root_excluding_prefixes(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: KnowledgeType,
    excluded_prefixes: &[String],
) -> Result<Vec<KnowledgeDirectoryConfigRecord>, String> {
    let directories = list_directories_with_app_root(working_dir, app_knowledge_dir, doc_type)?;
    let normalized_exclusions = excluded_prefixes
        .iter()
        .map(|prefix| normalize_relative_prefix(prefix))
        .collect::<Result<Vec<_>, _>>()?;
    let filtered = directories
        .into_iter()
        .filter(|directory| !path_matches_any_prefix(directory, &normalized_exclusions))
        .collect::<Vec<_>>();
    let mut records = Vec::with_capacity(filtered.len());
    for directory in filtered {
        records.push(read_directory_config_with_app_root(
            working_dir,
            app_knowledge_dir,
            doc_type,
            &directory,
        )?);
    }
    Ok(records)
}

pub(crate) fn score_document_text_match(
    query: &str,
    doc: &KnowledgeDocument,
) -> Option<(f32, String, Option<KnowledgeSearchMatchSection>)> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let terms = text_match_terms(&needle);

    let mut score = 0.0_f32;
    let mut snippet = String::new();
    let mut matched_section = None;

    let title = doc.title.to_lowercase();
    let path = doc.path.to_lowercase();
    let summary_text = active_summary(doc).unwrap_or_default();
    let rules_text = active_maintenance_rules(doc).unwrap_or_default();
    let summary = summary_text.to_lowercase();
    let rules = rules_text.to_lowercase();
    let body = doc.body.to_lowercase();

    if title.contains(&needle) {
        score += 8.0;
        snippet = doc.title.clone();
    } else if contains_all_text_match_terms(&title, &terms) {
        score += 5.5;
        snippet = doc.title.clone();
    }
    if path.contains(&needle) {
        score += 4.0;
        if snippet.is_empty() {
            snippet = doc.path.clone();
        }
    } else if contains_all_text_match_terms(&path, &terms) {
        score += 3.0;
        if snippet.is_empty() {
            snippet = doc.path.clone();
        }
    }
    if summary.contains(&needle) {
        score += 6.0;
        if snippet.is_empty() {
            snippet = summary_text.to_string();
        }
        matched_section = Some(KnowledgeSearchMatchSection::Summary);
    } else if contains_all_text_match_terms(&summary, &terms) {
        score += 4.5;
        if snippet.is_empty() {
            snippet = summary_text.to_string();
        }
        matched_section = Some(KnowledgeSearchMatchSection::Summary);
    }
    if rules.contains(&needle) {
        score += 4.0;
        if snippet.is_empty() {
            snippet = rules_text.to_string();
        }
        if matched_section.is_none() {
            matched_section = Some(KnowledgeSearchMatchSection::MaintenanceRules);
        }
    } else if contains_all_text_match_terms(&rules, &terms) {
        score += 3.0;
        if snippet.is_empty() {
            snippet = rules_text.to_string();
        }
        if matched_section.is_none() {
            matched_section = Some(KnowledgeSearchMatchSection::MaintenanceRules);
        }
    }
    if body.contains(&needle) {
        score += 3.0;
        if snippet.is_empty() {
            snippet = extract_snippet(&doc.body, &needle, &terms);
        }
        if matched_section.is_none() {
            matched_section = Some(KnowledgeSearchMatchSection::Body);
        }
    } else if contains_all_text_match_terms(&body, &terms) {
        score += 2.0;
        if snippet.is_empty() {
            snippet = extract_snippet(&doc.body, &needle, &terms);
        }
        if matched_section.is_none() {
            matched_section = Some(KnowledgeSearchMatchSection::Body);
        }
    }

    if score <= 0.0 {
        None
    } else {
        if snippet.is_empty() {
            snippet = active_summary(doc)
                .map(str::to_string)
                .unwrap_or_else(|| extract_snippet(&doc.body, &needle, &terms));
        }
        Some((score, snippet, matched_section))
    }
}

fn text_match_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for ch in query.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            terms.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        terms.push(current);
    }
    if terms.is_empty() && !query.is_empty() {
        terms.push(query.to_string());
    }
    terms.sort();
    terms.dedup();
    terms
}

fn contains_all_text_match_terms(text: &str, terms: &[String]) -> bool {
    !terms.is_empty() && terms.iter().all(|term| text.contains(term))
}

fn extract_snippet(text: &str, query: &str, terms: &[String]) -> String {
    let lowered = text.to_lowercase();
    let matched = std::iter::once(query)
        .chain(terms.iter().map(String::as_str))
        .find_map(|candidate| {
            if candidate.is_empty() {
                None
            } else {
                lowered
                    .find(candidate)
                    .map(|index| (index, candidate.len()))
            }
        });

    if let Some((index, matched_len)) = matched {
        let mut start = index.saturating_sub(80).min(text.len());
        while start > 0 && !text.is_char_boundary(start) {
            start -= 1;
        }

        let mut end = (index + matched_len + 120).min(text.len());
        while end < text.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        return text[start..end].trim().to_string();
    }

    text.lines().take(4).collect::<Vec<_>>().join("\n")
}

pub fn query_documents(
    working_dir: &str,
    query: &str,
    types: Option<&[KnowledgeType]>,
    limit: usize,
) -> Result<Vec<KnowledgeSearchHit>, String> {
    let mut items = Vec::new();
    let query_types = types
        .map(|values| values.to_vec())
        .unwrap_or_else(|| KnowledgeType::all().to_vec());

    for ty in query_types {
        let docs = list_documents(working_dir, Some(ty), None)?;
        for item in docs {
            let Ok(doc) = load_document_by_path(working_dir, ty, &item.path) else {
                continue;
            };
            let Some((score, snippet, matched_section)) = score_document_text_match(query, &doc)
            else {
                continue;
            };
            items.push(KnowledgeSearchHit {
                id: doc.id,
                doc_type: doc.doc_type,
                path: doc.path,
                title: doc.title,
                storage_source: doc.storage_source,
                inject_mode: doc.inject_mode,
                ai_maintained: doc.ai_maintained,
                score,
                snippet,
                matched_section,
                has_summary: doc.summary_enabled && has_summary_content(doc.summary.as_deref()),
                updated_at: doc.updated_at,
                match_kind: "lexical".to_string(),
                semantic_score: None,
                semantic_confidence: None,
                estimated_tokens: None,
            });
        }
    }

    items.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items.truncate(limit.max(1));
    Ok(items)
}

fn write_document_file(
    target_path: &Path,
    mut document: KnowledgeDocument,
) -> Result<KnowledgeDocument, String> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create knowledge document directory: {}", e))?;
    }
    if document.created_at == 0 {
        document.created_at = now_millis();
    }
    document.updated_at = now_millis();
    let rendered = render_document(&document)?;
    std::fs::write(target_path, rendered).map_err(|e| {
        format!(
            "Failed to write knowledge document '{}': {}",
            target_path.display(),
            e
        )
    })?;
    Ok(document)
}

pub fn save_document_to_path(
    target_path: &Path,
    mut document: KnowledgeDocument,
) -> Result<KnowledgeDocument, String> {
    document.path = normalize_relative_path(&document.path)?;
    sync_title_with_document_path(&mut document)?;
    resolve_document_inheritance(None, &mut document)?;
    apply_external_source_defaults(&mut document);
    apply_read_only_policy(&mut document);
    ensure_summary_state(&mut document);
    ensure_maintenance_rules(&mut document);
    ensure_skill_defaults(&mut document);
    validate_document(&document)?;
    write_document_file(target_path, document)
}

pub fn save_document(
    working_dir: &str,
    mut document: KnowledgeDocument,
) -> Result<KnowledgeDocument, String> {
    ensure_knowledge_roots(working_dir)?;
    document.path = normalize_relative_path(&document.path)?;
    sync_title_with_document_path(&mut document)?;
    resolve_document_inheritance(Some(working_dir), &mut document)?;
    apply_external_source_defaults(&mut document);
    apply_read_only_policy(&mut document);
    ensure_summary_state(&mut document);
    ensure_maintenance_rules(&mut document);
    ensure_skill_defaults(&mut document);
    validate_document(&document)?;

    let path = document_path(working_dir, document.doc_type, &document.path)?;
    write_document_file(&path, document)
}

pub fn prepare_document_preview(
    mut document: KnowledgeDocument,
) -> Result<KnowledgeDocument, String> {
    document.path = normalize_relative_path(&document.path)?;
    sync_title_with_document_path(&mut document)?;
    resolve_document_inheritance(None, &mut document)?;
    apply_external_source_defaults(&mut document);
    apply_read_only_policy(&mut document);
    ensure_summary_state(&mut document);
    ensure_maintenance_rules(&mut document);
    ensure_skill_defaults(&mut document);
    validate_document(&document)?;

    if document.created_at == 0 {
        document.created_at = now_millis();
    }
    if document.updated_at == 0 {
        document.updated_at = document.created_at;
    }

    Ok(document)
}

fn read_raw_document(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| {
        format!(
            "Failed to read knowledge document '{}': {}",
            path.display(),
            e
        )
    })
}

fn normalize_markdown(value: &str) -> String {
    value.replace("\r\n", "\n")
}

fn find_heading(markdown: &str, heading: &str, offset: usize) -> Option<usize> {
    let mut search_from = offset;
    while search_from < markdown.len() {
        let slice = &markdown[search_from..];
        let Some(relative) = slice.find(heading) else {
            return None;
        };
        let index = search_from + relative;
        let starts_line = index == 0 || markdown.as_bytes()[index - 1] == b'\n';
        let ends_line = markdown[index + heading.len()..]
            .chars()
            .next()
            .map(|ch| ch == '\n' || ch == '\r')
            .unwrap_or(true);
        if starts_line && ends_line {
            return Some(index);
        }
        search_from = index + heading.len();
    }
    None
}

fn heading_end(markdown: &str, start: usize) -> usize {
    markdown[start..]
        .find('\n')
        .map(|relative| start + relative + 1)
        .unwrap_or(markdown.len())
}

fn replace_title_heading(markdown: &str, title: &str) -> String {
    let normalized = normalize_markdown(markdown);
    let mut line_start = 0usize;
    while line_start < normalized.len() {
        let line_end = heading_end(&normalized, line_start);
        let line = normalized[line_start..line_end].trim_end_matches('\n');
        if line.starts_with("# ") {
            let mut out = String::new();
            out.push_str(&normalized[..line_start]);
            out.push_str("# ");
            out.push_str(title.trim());
            out.push('\n');
            out.push_str(&normalized[line_end..]);
            return out;
        }
        line_start = line_end;
    }

    let mut out = String::new();
    out.push_str("# ");
    out.push_str(title.trim());
    out.push_str("\n\n");
    out.push_str(normalized.trim_start_matches('\n'));
    out
}

fn replace_optional_section(
    markdown: &str,
    heading: &str,
    next_headings: &[&str],
    fallback_insert_before: &str,
    content: Option<&str>,
) -> String {
    let normalized = normalize_markdown(markdown);
    let new_content = content
        .map(str::trim_end)
        .filter(|value| !value.trim().is_empty());

    if let Some(start) = find_heading(&normalized, heading, 0) {
        let content_start = heading_end(&normalized, start);
        let end = next_headings
            .iter()
            .filter_map(|candidate| find_heading(&normalized, candidate, content_start))
            .min()
            .unwrap_or(normalized.len());

        let mut out = String::new();
        let prefix = normalized[..start].trim_end_matches('\n');
        out.push_str(prefix);
        if let Some(value) = new_content {
            if !prefix.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(heading);
            out.push('\n');
            out.push_str(value);
            out.push_str("\n\n");
        } else if !prefix.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(normalized[end..].trim_start_matches('\n'));
        return out;
    }

    let Some(value) = new_content else {
        return normalized;
    };
    let insert_at =
        find_heading(&normalized, fallback_insert_before, 0).unwrap_or(normalized.len());
    let prefix = normalized[..insert_at].trim_end_matches('\n');
    let suffix = normalized[insert_at..].trim_start_matches('\n');

    let mut out = String::new();
    out.push_str(prefix);
    if !prefix.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(heading);
    out.push('\n');
    out.push_str(value);
    out.push_str("\n\n");
    out.push_str(suffix);
    out
}

fn replace_content_section(markdown: &str, content: &str) -> Result<String, String> {
    let normalized = normalize_markdown(markdown);
    let Some(start) = find_heading(&normalized, "## Content", 0) else {
        return Err("Knowledge document missing ## Content section".to_string());
    };

    let prefix = normalized[..start].trim_end_matches('\n');
    let mut out = String::new();
    out.push_str(prefix);
    if !prefix.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str("## Content\n");
    out.push_str(content.trim_end());
    out.push('\n');
    Ok(out)
}

fn has_explicit_content_section(markdown: &str) -> bool {
    let normalized = normalize_markdown(markdown);
    find_heading(&normalized, "## Content", 0).is_some()
}

fn write_document_preserving_layout(
    target_path: &Path,
    document: &KnowledgeDocument,
    body_markdown: String,
) -> Result<(), String> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create knowledge document directory: {}", e))?;
    }

    let mut rendered = String::new();
    rendered.push_str("---\n");
    rendered.push_str(&render_frontmatter(document)?);
    rendered.push_str("---\n\n");
    rendered.push_str(body_markdown.trim_start_matches('\n'));
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }

    std::fs::write(target_path, rendered).map_err(|e| {
        format!(
            "Failed to write knowledge document '{}': {}",
            target_path.display(),
            e
        )
    })
}

fn locate_document(
    working_dir: &str,
    request: &KnowledgeUpdateRequest,
) -> Result<KnowledgeDocument, String> {
    if let Some(id) = request.id.as_ref() {
        if let Some(doc) = load_document_by_id(working_dir, id)? {
            return Ok(doc);
        }
    }
    let doc_type = request
        .doc_type
        .or_else(|| guess_type_from_path(&request.path))
        .ok_or_else(|| "knowledge update requires type or a type-prefixed path".to_string())?;
    load_document_by_path(working_dir, doc_type, &request.path)
}

pub fn update_document(
    working_dir: &str,
    request: KnowledgeUpdateRequest,
) -> Result<KnowledgeDocument, String> {
    ensure_knowledge_roots(working_dir)?;

    match request.op {
        KnowledgeUpdateOp::Create => {
            let doc_type = request
                .doc_type
                .ok_or_else(|| "create requires type".to_string())?;
            let title = request
                .title
                .ok_or_else(|| "create requires title".to_string())?;
            let inherit_inject_mode = request
                .inherit_inject_mode
                .unwrap_or(request.inject_mode.is_none());
            let inject_mode = request
                .inject_mode
                .unwrap_or_else(|| default_document_inject_mode_for_type(doc_type));
            let inherit_ai_config = request.inherit_ai_config.unwrap_or(
                request.ai_maintained.is_none()
                    && request.explicit_maintenance_rules.is_none()
                    && request.maintenance_rules.is_none(),
            );
            let ai_maintained = request
                .ai_maintained
                .unwrap_or_else(|| default_ai_maintained_for_type(doc_type));
            let explicit_maintenance_rules = request
                .explicit_maintenance_rules
                .unwrap_or_else(|| default_explicit_maintenance_rules_for_type(doc_type));
            let external_source = request.external_source.unwrap_or(None);
            let summary = request.summary.and_then(|value| value);
            let summary_enabled = request.summary_enabled.unwrap_or_else(|| {
                has_summary_content(summary.as_deref())
                    || default_summary_enabled_for_type(doc_type)
            });
            let body = request
                .body
                .and_then(|value| value)
                .ok_or_else(|| "create requires body".to_string())?;
            let maintenance_rules = request.maintenance_rules.and_then(|value| value);
            let mut doc = KnowledgeDocument {
                id: request
                    .id
                    .unwrap_or_else(|| format!("kd_{}", uuid::Uuid::new_v4())),
                doc_type,
                path: normalize_relative_path(&request.path)?,
                title,
                inject_mode,
                inherit_inject_mode,
                inject_mode_source: if inherit_inject_mode {
                    type_default_config_source()
                } else {
                    self_config_source()
                },
                summary_enabled,
                command_enabled: request.command_enabled.unwrap_or(false),
                read_only: request.read_only.unwrap_or(false),
                ai_maintained,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config,
                ai_config_source: if inherit_ai_config {
                    type_default_config_source()
                } else {
                    self_config_source()
                },
                explicit_maintenance_rules,
                external_source,
                skill_enabled: request.skill_enabled,
                skill_surface: request.skill_surface,
                command_trigger: request.command_trigger.and_then(|value| value),
                argument_hint: request.argument_hint.and_then(|value| value),
                tools: Vec::new(),
                summary,
                body,
                maintenance_rules,
                created_at: now_millis(),
                updated_at: now_millis(),
            };
            if doc.doc_type == KnowledgeType::Skill
                && request.command_enabled.is_some()
                && request.skill_enabled.is_none()
                && request.skill_surface.is_none()
            {
                apply_skill_command_enabled(&mut doc, request.command_enabled.unwrap_or(false));
            }
            ensure_maintenance_rules(&mut doc);
            ensure_skill_defaults(&mut doc);
            save_document(working_dir, doc)
        }
        KnowledgeUpdateOp::Edit => edit_document(
            working_dir,
            &request.path,
            request.doc_type,
            KnowledgeDocumentPatch {
                id: request.id,
                doc_type: request.doc_type,
                title: request.title,
                inject_mode: request.inject_mode,
                inherit_inject_mode: request.inherit_inject_mode,
                summary_enabled: request.summary_enabled,
                command_enabled: request.command_enabled,
                read_only: request.read_only,
                ai_maintained: request.ai_maintained,
                inherit_ai_config: request.inherit_ai_config,
                explicit_maintenance_rules: request.explicit_maintenance_rules,
                external_source: request.external_source,
                skill_enabled: request.skill_enabled,
                skill_surface: request.skill_surface,
                command_trigger: request.command_trigger,
                argument_hint: request.argument_hint,
                summary: request.summary,
                body: request.body,
                maintenance_rules: request.maintenance_rules,
                new_path: request.new_path,
            },
        ),
        KnowledgeUpdateOp::Delete => {
            let doc = locate_document(working_dir, &request)?;
            if doc.read_only {
                return Err("Cannot delete a read-only knowledge document".to_string());
            }
            let path = document_path(working_dir, doc.doc_type, &doc.path)?;
            std::fs::remove_file(&path).map_err(|e| {
                format!(
                    "Failed to delete knowledge document '{}': {}",
                    path.display(),
                    e
                )
            })?;
            Ok(doc)
        }
        KnowledgeUpdateOp::UpdateMeta => {
            let mut doc = locate_document(working_dir, &request)?;
            if doc.read_only
                && (is_read_only_locked_by_source(&doc) || request.read_only != Some(false))
            {
                return Err("Cannot update a read-only knowledge document".to_string());
            }
            let old_type = doc.doc_type;
            let old_path = doc.path.clone();
            if let Some(doc_type) = request.doc_type {
                doc.doc_type = doc_type;
            }
            if let Some(inject_mode) = request.inject_mode {
                doc.inject_mode = inject_mode;
            }
            if let Some(inherit_inject_mode) = request.inherit_inject_mode {
                doc.inherit_inject_mode = inherit_inject_mode;
            } else if request.inject_mode.is_some() {
                doc.inherit_inject_mode = false;
            }
            if let Some(summary_enabled) = request.summary_enabled {
                doc.summary_enabled = summary_enabled;
            }
            if let Some(skill_enabled) = request.skill_enabled {
                doc.skill_enabled = Some(skill_enabled);
            }
            if let Some(skill_surface) = request.skill_surface {
                doc.skill_surface = Some(skill_surface);
            }
            if let Some(command_trigger) = request.command_trigger {
                doc.command_trigger = command_trigger;
            }
            if let Some(argument_hint) = request.argument_hint {
                doc.argument_hint = argument_hint;
            }
            if let Some(command_enabled) = request.command_enabled {
                if doc.doc_type == KnowledgeType::Skill
                    && request.skill_enabled.is_none()
                    && request.skill_surface.is_none()
                {
                    apply_skill_command_enabled(&mut doc, command_enabled);
                } else {
                    doc.command_enabled = command_enabled;
                }
            }
            if let Some(read_only) = request.read_only {
                doc.read_only = read_only;
            }
            if let Some(ai_maintained) = request.ai_maintained {
                doc.ai_maintained = ai_maintained;
            }
            if let Some(inherit_ai_config) = request.inherit_ai_config {
                doc.inherit_ai_config = inherit_ai_config;
            } else if request.ai_maintained.is_some()
                || request.explicit_maintenance_rules.is_some()
                || request.maintenance_rules.is_some()
            {
                doc.inherit_ai_config = false;
            }
            if let Some(explicit_maintenance_rules) = request.explicit_maintenance_rules {
                doc.explicit_maintenance_rules = explicit_maintenance_rules;
            }
            if let Some(external_source) = request.external_source {
                doc.external_source = external_source;
            }
            if let Some(new_path) = request.new_path {
                sync_title_after_path_change(&mut doc, &new_path)?;
            }
            if let Some(id) = request.id {
                doc.id = id;
            }
            ensure_summary_state(&mut doc);
            ensure_maintenance_rules(&mut doc);
            ensure_skill_defaults(&mut doc);
            let saved = save_document(working_dir, doc)?;
            if saved.doc_type != old_type || saved.path != old_path {
                let old_file = document_path(working_dir, old_type, &old_path)?;
                if old_file.is_file() {
                    let _ = std::fs::remove_file(old_file);
                }
            }
            Ok(saved)
        }
        KnowledgeUpdateOp::UpdateSummary => {
            let mut doc = locate_document(working_dir, &request)?;
            if doc.read_only {
                return Err("Cannot update a read-only knowledge document".to_string());
            }
            doc.summary = request.summary.and_then(|value| value);
            ensure_summary_state(&mut doc);
            doc.updated_at = now_millis();
            validate_document(&doc)?;

            let path = document_path(working_dir, doc.doc_type, &doc.path)?;
            let raw = read_raw_document(&path)?;
            let (_, body) = split_frontmatter(&raw)?;
            let updated_body = if doc.doc_type == KnowledgeType::Memory {
                render_document_body(&doc)?
            } else if has_explicit_content_section(body) {
                let with_title = replace_title_heading(body, &doc.title);
                replace_optional_section(
                    &with_title,
                    "## Summary",
                    &["## Maintenance Rules", "## Content"],
                    "## Content",
                    active_summary(&doc),
                )
            } else {
                render_document_body(&doc)?
            };
            write_document_preserving_layout(&path, &doc, updated_body)?;
            Ok(doc)
        }
        KnowledgeUpdateOp::UpdateBody => {
            let mut doc = locate_document(working_dir, &request)?;
            if doc.read_only {
                return Err("Cannot update a read-only knowledge document".to_string());
            }
            doc.body = request
                .body
                .and_then(|value| value)
                .ok_or_else(|| "update_body requires body".to_string())?;
            doc.updated_at = now_millis();
            validate_document(&doc)?;

            let path = document_path(working_dir, doc.doc_type, &doc.path)?;
            let raw = read_raw_document(&path)?;
            let (_, body) = split_frontmatter(&raw)?;
            let updated_body = if doc.doc_type == KnowledgeType::Memory {
                render_document_body(&doc)?
            } else if has_explicit_content_section(body) {
                let with_title = replace_title_heading(body, &doc.title);
                replace_content_section(&with_title, &doc.body)?
            } else {
                render_document_body(&doc)?
            };
            write_document_preserving_layout(&path, &doc, updated_body)?;
            Ok(doc)
        }
        KnowledgeUpdateOp::UpdateRules => {
            let mut doc = locate_document(working_dir, &request)?;
            if doc.read_only {
                return Err("Cannot update a read-only knowledge document".to_string());
            }
            doc.maintenance_rules = request.maintenance_rules.and_then(|value| value);
            ensure_maintenance_rules(&mut doc);
            doc.updated_at = now_millis();
            validate_document(&doc)?;

            let path = document_path(working_dir, doc.doc_type, &doc.path)?;
            let raw = read_raw_document(&path)?;
            let (_, body) = split_frontmatter(&raw)?;
            let updated_body = if doc.doc_type == KnowledgeType::Memory {
                render_document_body(&doc)?
            } else if has_explicit_content_section(body) {
                let with_title = replace_title_heading(body, &doc.title);
                replace_optional_section(
                    &with_title,
                    "## Maintenance Rules",
                    &["## Content"],
                    "## Content",
                    active_maintenance_rules(&doc),
                )
            } else {
                render_document_body(&doc)?
            };
            write_document_preserving_layout(&path, &doc, updated_body)?;
            Ok(doc)
        }
    }
}

pub fn read_document(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
    part: &str,
) -> Result<KnowledgeReadResult, String> {
    let normalized_path = normalize_relative_path(path)?;
    let document = load_document_by_path(working_dir, doc_type, &normalized_path)?;
    let file_path = document_path(working_dir, doc_type, &normalized_path).ok();
    let file_metadata = file_path
        .as_deref()
        .map(|value| build_document_file_metadata(Some(working_dir), value, &document));
    build_read_result(document, part, file_metadata)
}

pub fn read_document_from_root(
    knowledge_root: &Path,
    doc_type: KnowledgeType,
    path: &str,
    part: &str,
) -> Result<KnowledgeReadResult, String> {
    let normalized_path = normalize_relative_path(path)?;
    let document = load_document_by_root(knowledge_root, doc_type, &normalized_path)?;
    let file_path = document_path_in_root(knowledge_root, doc_type, &normalized_path).ok();
    let file_metadata = file_path
        .as_deref()
        .map(|value| build_document_file_metadata(None, value, &document));
    build_read_result(document, part, file_metadata)
}

pub fn read_document_part(
    working_dir: &str,
    doc_type: KnowledgeType,
    path: &str,
    part: &str,
) -> Result<String, String> {
    let document = load_document_by_path(working_dir, doc_type, path)?;
    let resolved_part = normalize_read_part(part)?;
    Ok(match resolved_part {
        "full" => render_document_body(&document)?,
        "summary" => document.summary.unwrap_or_default(),
        "body" => document.body,
        _ => unreachable!("normalize_read_part only returns known values"),
    })
}

fn normalize_read_part(part: &str) -> Result<&'static str, String> {
    match part.trim() {
        "" | "full" => Ok("full"),
        "summary" => Ok("summary"),
        "body" => Ok("body"),
        other => Err(format!(
            "knowledge_read part must be one of: full, summary, body (got '{}')",
            other
        )),
    }
}

fn build_read_result(
    document: KnowledgeDocument,
    part: &str,
    file_metadata: Option<KnowledgeDocumentFileMetadata>,
) -> Result<KnowledgeReadResult, String> {
    let resolved_part = normalize_read_part(part)?;
    let mut document = document;

    match resolved_part {
        "full" => {}
        "summary" => {
            document.body.clear();
            document.maintenance_rules = None;
            document.explicit_maintenance_rules = false;
        }
        "body" => {
            document.summary = None;
            document.summary_enabled = false;
            document.maintenance_rules = None;
            document.explicit_maintenance_rules = false;
        }
        _ => unreachable!("normalize_read_part only returns known values"),
    }

    Ok(KnowledgeReadResult {
        document,
        part: resolved_part.to_string(),
        file_metadata,
    })
}

fn resolve_document_file_path_with_app_root(
    working_dir: &str,
    app_knowledge_dir: Option<&PathBuf>,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<PathBuf, String> {
    let workspace_path = document_path(working_dir, doc_type, path)?;
    if workspace_path.is_file() {
        return Ok(workspace_path);
    }

    if let Some(app_root) = app_knowledge_dir {
        let app_path = document_path_in_root(app_root, doc_type, path)?;
        if app_path.is_file() {
            return Ok(app_path);
        }
    }

    Ok(workspace_path)
}

fn build_document_file_metadata(
    working_dir: Option<&str>,
    file_path: &Path,
    document: &KnowledgeDocument,
) -> KnowledgeDocumentFileMetadata {
    let rendered = render_document(document).ok();
    let byte_size = rendered
        .as_ref()
        .map(|value| value.as_bytes().len() as u64)
        .or_else(|| rendered_document_size_bytes(document).ok());
    let line_count = rendered.as_ref().map(|value| value.lines().count() as u64);
    let char_count = rendered.as_ref().map(|value| value.chars().count() as u64);
    let modified_at = std::fs::metadata(file_path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64);
    let (last_commit_author, last_commit_at) = working_dir
        .and_then(|value| read_last_commit_metadata(value, file_path))
        .unwrap_or((None, None));

    KnowledgeDocumentFileMetadata {
        byte_size,
        line_count,
        char_count,
        estimated_tokens: Some(estimate_document_tokens_for_metadata(document)),
        modified_at,
        last_commit_author,
        last_commit_at,
    }
}

fn estimate_document_tokens_for_metadata(document: &KnowledgeDocument) -> u64 {
    let mut text = String::new();
    if let Some(summary) = active_summary(document) {
        text.push_str(summary);
        text.push('\n');
    }
    if let Some(rules) = active_maintenance_rules(document) {
        text.push_str(rules);
        text.push('\n');
    }
    text.push_str(&document.body);
    estimate_tokens_from_metadata_text(&text)
}

fn estimate_tokens_from_metadata_text(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    ((text.as_bytes().len() as f64) / 3.5).ceil() as u64
}

fn read_last_commit_metadata(
    working_dir: &str,
    file_path: &Path,
) -> Option<(Option<String>, Option<i64>)> {
    let trimmed_working_dir = working_dir.trim();
    if trimmed_working_dir.is_empty() {
        return None;
    }

    let relative_path = file_path
        .strip_prefix(Path::new(trimmed_working_dir))
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    if relative_path.is_empty() {
        return None;
    }

    let output = crate::process_util::command("git")
        .args([
            "-c",
            "core.quotePath=false",
            "log",
            "-1",
            "--follow",
            "--format=%an%x00%at",
            "--",
            &relative_path,
        ])
        .current_dir(trimmed_working_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let record = stdout.trim();
    if record.is_empty() {
        return None;
    }

    let mut parts = record.split('\0');
    let author = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let committed_at = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.saturating_mul(1000));

    if author.is_none() && committed_at.is_none() {
        return None;
    }

    Some((author, committed_at))
}

pub fn infer_type_from_path(path: &str) -> Option<KnowledgeType> {
    guess_type_from_path(path)
}

pub fn parse_document_content(
    content: &str,
    path_hint: Option<&str>,
) -> Result<KnowledgeDocument, String> {
    parse_document(content, path_hint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_doc() -> KnowledgeDocument {
        KnowledgeDocument {
            id: "kd_test".to_string(),
            doc_type: KnowledgeType::Design,
            path: "gameplay/core-loop.md".to_string(),
            title: "Core Loop".to_string(),
            inject_mode: KnowledgeInjectMode::Excerpt,
            inherit_inject_mode: false,
            inject_mode_source: self_config_source(),
            summary_enabled: true,
            command_enabled: true,
            read_only: false,
            ai_maintained: false,
            storage_source: KnowledgeStorageSource::Project,
            inherit_ai_config: false,
            ai_config_source: self_config_source(),
            explicit_maintenance_rules: false,
            external_source: None,
            skill_enabled: None,
            skill_surface: None,
            command_trigger: None,
            argument_hint: None,
            tools: Vec::new(),
            summary: Some("Short summary".to_string()),
            body: "Body content".to_string(),
            maintenance_rules: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn sample_directory_config() -> KnowledgeDirectoryConfig {
        KnowledgeDirectoryConfig {
            version: 4,
            summary: "Maintain subsystem structure cache".to_string(),
            inject_mode: KnowledgeInjectMode::Excerpt,
            inherit_inject_mode: false,
            ai_maintained: true,
            inherit_ai_config: false,
            explicit_maintenance_rules: true,
            lexical_search: FolderIndexRuleSetting::Enabled,
            vector_search: FolderIndexRuleSetting::Disabled,
            inherit_to_children: true,
            allow_create_documents: true,
            allow_create_directories: true,
            allow_move_documents: true,
            allow_move_directories: true,
            maintenance_rules: "- Keep verified structure facts only".to_string(),
        }
    }

    fn sample_unity_bundle_doc() -> KnowledgeDocument {
        KnowledgeDocument {
            id: "kd_unity_bundle_doc".to_string(),
            doc_type: KnowledgeType::Reference,
            path: "unity-official-docs/manual/ExecutionOrder.md".to_string(),
            title: "Execution Order".to_string(),
            inject_mode: KnowledgeInjectMode::None,
            inherit_inject_mode: false,
            inject_mode_source: self_config_source(),
            summary_enabled: true,
            command_enabled: false,
            read_only: true,
            ai_maintained: false,
            storage_source: KnowledgeStorageSource::Project,
            inherit_ai_config: false,
            ai_config_source: self_config_source(),
            explicit_maintenance_rules: false,
            external_source: Some(KnowledgeExternalSource {
                provider: KnowledgeSourceProvider::Unity,
                locator: Some(
                    "https://docs.unity3d.com/2022.3/Documentation/Manual/ExecutionOrder.html"
                        .to_string(),
                ),
                source_id: Some("unity-2022.3".to_string()),
                sync_enabled: true,
            }),
            skill_enabled: None,
            skill_surface: None,
            command_trigger: None,
            argument_hint: None,
            tools: Vec::new(),
            summary: Some("Execution order summary".to_string()),
            body: "Execution order body".to_string(),
            maintenance_rules: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn sample_memory_doc() -> KnowledgeDocument {
        KnowledgeDocument {
            id: "kd_memory_test".to_string(),
            doc_type: KnowledgeType::Memory,
            path: MEMORY_USER_PREFERENCE_PATH.to_string(),
            title: "用户偏好".to_string(),
            inject_mode: KnowledgeInjectMode::Full,
            inherit_inject_mode: false,
            inject_mode_source: self_config_source(),
            summary_enabled: false,
            command_enabled: false,
            read_only: false,
            ai_maintained: false,
            storage_source: KnowledgeStorageSource::Project,
            inherit_ai_config: false,
            ai_config_source: self_config_source(),
            explicit_maintenance_rules: true,
            external_source: None,
            skill_enabled: None,
            skill_surface: None,
            command_trigger: None,
            argument_hint: None,
            tools: Vec::new(),
            summary: None,
            body: "# 输出方式\n## 细节\n- 先给答案".to_string(),
            maintenance_rules: Some("- 直接给结论".to_string()),
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn read_document_part_modes_return_expected_sections() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let mut doc = sample_doc();
        doc.explicit_maintenance_rules = true;
        doc.maintenance_rules = Some("Keep only durable notes".to_string());
        save_document(&working_dir, doc).expect("save");

        let full = read_document(
            &working_dir,
            KnowledgeType::Design,
            "gameplay/core-loop.md",
            "full",
        )
        .expect("read full");
        assert_eq!(full.part, "full");
        assert_eq!(full.document.summary.as_deref(), Some("Short summary"));
        assert_eq!(full.document.body, "Body content");
        assert_eq!(
            full.document.maintenance_rules.as_deref(),
            Some("Keep only durable notes")
        );
        assert!(full.document.summary_enabled);
        assert!(full.document.explicit_maintenance_rules);

        let summary = read_document(
            &working_dir,
            KnowledgeType::Design,
            "gameplay/core-loop.md",
            "summary",
        )
        .expect("read summary");
        assert_eq!(summary.part, "summary");
        assert_eq!(summary.document.summary.as_deref(), Some("Short summary"));
        assert!(summary.document.body.is_empty());
        assert!(summary.document.maintenance_rules.is_none());
        assert!(!summary.document.explicit_maintenance_rules);

        let body = read_document(
            &working_dir,
            KnowledgeType::Design,
            "gameplay/core-loop.md",
            "body",
        )
        .expect("read body");
        assert_eq!(body.part, "body");
        assert!(body.document.summary.is_none());
        assert!(!body.document.summary_enabled);
        assert_eq!(body.document.body, "Body content");
        assert!(body.document.maintenance_rules.is_none());
        assert!(!body.document.explicit_maintenance_rules);
    }

    #[test]
    fn read_document_rejects_unknown_part() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        save_document(&working_dir, sample_doc()).expect("save");

        let error = read_document(
            &working_dir,
            KnowledgeType::Design,
            "gameplay/core-loop.md",
            "auto",
        )
        .expect_err("auto should be rejected");

        assert!(error.contains("full, summary, body"));
    }

    #[test]
    fn render_and_parse_round_trip() {
        let rendered = render_document(&sample_doc()).expect("render");
        let parsed = parse_document(&rendered, Some("gameplay/core-loop.md")).expect("parse");
        assert_eq!(parsed.title, "core-loop");
        assert_eq!(parsed.summary.as_deref(), Some("Short summary"));
        assert!(parsed.summary_enabled);
        assert_eq!(parsed.body, "Body content");
    }

    #[test]
    fn parse_document_preserves_h1_inside_content_section() {
        let mut doc = sample_doc();
        doc.body = "# 输出方式\n- 直接给结论\n\n## 细节\n- 保留实现说明".to_string();

        let rendered = render_document(&doc).expect("render");
        let parsed = parse_document(&rendered, Some("gameplay/core-loop.md")).expect("parse");

        assert_eq!(
            parsed.body,
            "# 输出方式\n- 直接给结论\n\n## 细节\n- 保留实现说明"
        );
    }

    #[test]
    fn parse_document_recovers_body_when_content_heading_is_missing() {
        let raw = render_document(&sample_doc())
            .expect("render")
            .replace("## Content\nBody content\n", "# 输出方式\n- 直接给结论\n");

        let parsed = parse_document(&raw, Some("gameplay/core-loop.md")).expect("parse");

        assert_eq!(parsed.title, "core-loop");
        assert_eq!(parsed.summary.as_deref(), Some("Short summary"));
        assert_eq!(parsed.body, "# 输出方式\n- 直接给结论");
    }

    #[test]
    fn render_and_parse_memory_round_trip_uses_comment_blocks() {
        let rendered = render_document(&sample_memory_doc()).expect("render memory");

        assert!(rendered.contains(MEMORY_MAINTAIN_RULES_START));
        assert!(rendered.contains(MEMORY_MAINTAIN_RULES_END));
        assert!(rendered.contains(MEMORY_BODY_START));
        assert!(rendered.contains(MEMORY_BODY_END));
        assert!(!rendered.contains("## Maintenance Rules"));
        assert!(!rendered.contains("## Content"));

        let parsed =
            parse_document(&rendered, Some(MEMORY_USER_PREFERENCE_PATH)).expect("parse memory");
        assert_eq!(parsed.doc_type, KnowledgeType::Memory);
        assert_eq!(parsed.body, "# 输出方式\n## 细节\n- 先给答案");
        assert_eq!(parsed.maintenance_rules.as_deref(), Some("- 直接给结论"));
    }

    #[test]
    fn parse_memory_document_keeps_legacy_heading_format_compatible() {
        let raw = r#"---
id: kd_memory_test
type: memory
path: user-preference.md
title: 用户偏好
scope: user
injectMode: full
summaryEnabled: false
commandEnabled: false
readOnly: false
aiMaintained: false
explicitMaintenanceRules: true
createdAt: 1
updatedAt: 1
---

# 用户偏好

## Maintenance Rules
- 直接给结论

## Content
# 输出方式
## 细节
- 先给答案
"#;

        let parsed = parse_document(raw, Some(MEMORY_USER_PREFERENCE_PATH)).expect("parse legacy");
        assert_eq!(parsed.doc_type, KnowledgeType::Memory);
        assert_eq!(parsed.body, "# 输出方式\n## 细节\n- 先给答案");
        assert_eq!(parsed.maintenance_rules.as_deref(), Some("- 直接给结论"));
    }

    #[test]
    fn parse_legacy_document_enables_summary_when_section_exists() {
        let rendered = r#"---
id: kd_test
type: design
path: gameplay/core-loop.md
title: Core Loop
scope: project
injectMode: excerpt
commandEnabled: true
readOnly: false
aiMaintained: false
createdAt: 1
updatedAt: 1
---

# Core Loop

## Summary
Short summary

## Content
Body content
"#;

        let parsed = parse_document(rendered, Some("gameplay/core-loop.md")).expect("parse");
        assert!(parsed.summary_enabled);
        assert_eq!(parsed.summary.as_deref(), Some("Short summary"));
    }

    #[test]
    fn render_and_parse_skill_round_trip_preserves_skill_fields() {
        let doc = KnowledgeDocument {
            id: "kd_skill".to_string(),
            doc_type: KnowledgeType::Skill,
            path: "create-skill.md".to_string(),
            title: "Create Skill".to_string(),
            inject_mode: KnowledgeInjectMode::None,
            inherit_inject_mode: false,
            inject_mode_source: self_config_source(),
            summary_enabled: true,
            command_enabled: true,
            read_only: false,
            ai_maintained: false,
            storage_source: KnowledgeStorageSource::Project,
            inherit_ai_config: false,
            ai_config_source: self_config_source(),
            explicit_maintenance_rules: false,
            external_source: None,
            skill_enabled: Some(true),
            skill_surface: Some(SkillSurface::Both),
            command_trigger: Some("/create-skill".to_string()),
            argument_hint: Some("<skill-name>".to_string()),
            tools: vec!["skill_create".to_string(), "skill_reload".to_string()],
            summary: Some("Create a new Skill.".to_string()),
            body: "## Instructions\n\n1. Do the work.".to_string(),
            maintenance_rules: None,
            created_at: 1,
            updated_at: 1,
        };

        let rendered = render_document(&doc).expect("render");
        let parsed = parse_document(&rendered, Some("create-skill.md")).expect("parse");
        assert_eq!(parsed.skill_enabled, Some(true));
        assert_eq!(parsed.skill_surface, Some(SkillSurface::Both));
        assert_eq!(parsed.command_trigger.as_deref(), Some("/create-skill"));
        assert_eq!(parsed.argument_hint.as_deref(), Some("<skill-name>"));
        assert_eq!(parsed.tools, vec!["skill_create", "skill_reload"]);
        assert_eq!(parsed.summary.as_deref(), Some("Create a new Skill."));
    }

    #[test]
    fn read_only_is_derived_from_external_source_provider() {
        let mut doc = sample_doc();
        doc.read_only = false;
        doc.external_source = Some(KnowledgeExternalSource {
            provider: KnowledgeSourceProvider::Feishu,
            locator: Some("space://design".to_string()),
            source_id: Some("src_1".to_string()),
            sync_enabled: true,
        });

        let rendered = render_document(&doc).expect("render");
        assert!(!rendered.contains("\nscope:"));
        let parsed = parse_document(&rendered, Some("gameplay/core-loop.md")).expect("parse");
        assert!(parsed.read_only);
    }

    #[test]
    fn legacy_scope_frontmatter_is_ignored() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let doc = sample_doc();
        let raw = render_document(&doc)
            .expect("render")
            .replace("title: Core Loop\n", "title: Core Loop\nscope: external\n");

        let parsed = parse_document(&raw, Some("gameplay/core-loop.md")).expect("parse");
        save_document(&working_dir, parsed).expect("save legacy scope doc");
        let rewritten = std::fs::read_to_string(
            document_path(&working_dir, KnowledgeType::Design, "gameplay/core-loop.md")
                .expect("document path"),
        )
        .expect("read rewritten doc");

        assert!(!rewritten.contains("\nscope:"));
    }

    #[test]
    fn parse_feishu_external_document_strips_legacy_import_header() {
        let rendered = r#"---
id: kd_feishu_doc
type: reference
path: feishu-knowledge-base/gameplay/battle-design.md
title: 战斗设计改进草案
scope: external
injectMode: none
summaryEnabled: false
commandEnabled: false
readOnly: true
aiMaintained: false
explicitMaintenanceRules: false
externalSource:
  provider: feishu
  locator: space:test;node:test;obj:test
  sourceId: node_test
  syncEnabled: true
createdAt: 1
updatedAt: 1
---

## Content
# 战斗设计改进草案

来源：飞书知识库 / Example Knowledge Space
节点：`node_example_docx`
对象类型：`docx`

---

## DEMO版本问题

- 能量循环没有实装
"#;

        let parsed = parse_document(
            rendered,
            Some("feishu-knowledge-base/gameplay/battle-design.md"),
        )
        .expect("parse");

        assert_eq!(
            parsed.body,
            "## DEMO版本问题\n\n- 能量循环没有实装".to_string()
        );
    }

    #[test]
    fn parse_unity_external_document_disables_summary_even_when_frontmatter_enables_it() {
        let rendered = r#"---
id: kd_unity_doc
type: reference
path: unity-official-docs/manual/ExecutionOrder.md
title: Execution Order
scope: external
injectMode: none
summaryEnabled: true
commandEnabled: false
readOnly: true
aiMaintained: false
explicitMaintenanceRules: false
externalSource:
  provider: unity
  locator: https://docs.unity3d.com/2022.3/Documentation/Manual/ExecutionOrder.html
  sourceId: unity-2022.3
  syncEnabled: true
createdAt: 1
updatedAt: 1
---

# Execution Order

## Summary
Execution order summary

## Content
Execution order body
"#;

        let parsed = parse_document(
            rendered,
            Some("unity-official-docs/manual/ExecutionOrder.md"),
        )
        .expect("parse");

        assert!(!parsed.summary_enabled);
        assert_eq!(parsed.summary.as_deref(), Some("Execution order summary"));
    }

    #[test]
    fn stored_read_only_flag_is_preserved_for_user_managed_sources() {
        let rendered = r#"---
id: kd_test
type: design
path: gameplay/core-loop.md
title: Core Loop
scope: external
injectMode: excerpt
commandEnabled: true
readOnly: true
aiMaintained: false
externalSource:
  provider: custom
  locator: manual://note
  sourceId: src_custom
  syncEnabled: false
createdAt: 1
updatedAt: 1
---

# Core Loop

## Summary
Short summary

## Content
Body content
"#;

        let parsed = parse_document(rendered, Some("gameplay/core-loop.md")).expect("parse");
        assert!(parsed.read_only);
    }

    #[test]
    fn update_body_preserves_meta() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        save_document(&working_dir, sample_doc()).expect("save");
        let updated = update_document(
            &working_dir,
            KnowledgeUpdateRequest {
                op: KnowledgeUpdateOp::UpdateBody,
                path: "gameplay/core-loop.md".to_string(),
                doc_type: Some(KnowledgeType::Design),
                body: Some(Some("New body".to_string())),
                ..Default::default()
            },
        )
        .expect("update");
        assert_eq!(updated.body, "New body");
        let reread =
            load_document_by_path(&working_dir, KnowledgeType::Design, "gameplay/core-loop.md")
                .unwrap();
        assert_eq!(reread.summary.as_deref(), Some("Short summary"));
    }

    #[test]
    fn update_body_rewrites_malformed_document_with_canonical_content_section() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let raw = render_document(&sample_doc())
            .expect("render")
            .replace("## Content\nBody content\n", "# 输出方式\n- 直接给结论\n");
        let path = document_path(&working_dir, KnowledgeType::Design, "gameplay/core-loop.md")
            .expect("document path");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        std::fs::write(&path, raw).expect("write malformed document");

        let updated = update_document(
            &working_dir,
            KnowledgeUpdateRequest {
                op: KnowledgeUpdateOp::UpdateBody,
                path: "gameplay/core-loop.md".to_string(),
                doc_type: Some(KnowledgeType::Design),
                body: Some(Some("# 交付方式\n- 先给结论\n- 再补依据".to_string())),
                ..Default::default()
            },
        )
        .expect("update malformed document");

        assert_eq!(updated.body, "# 交付方式\n- 先给结论\n- 再补依据");

        let rewritten = std::fs::read_to_string(&path).expect("read rewritten document");
        assert!(rewritten.contains("## Content\n# 交付方式\n- 先给结论\n- 再补依据\n"));

        let reread =
            load_document_by_path(&working_dir, KnowledgeType::Design, "gameplay/core-loop.md")
                .expect("reload");
        assert_eq!(reread.summary.as_deref(), Some("Short summary"));
        assert_eq!(reread.body, "# 交付方式\n- 先给结论\n- 再补依据");
    }

    #[test]
    fn update_memory_body_rewrites_document_with_comment_blocks() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        save_document(&working_dir, sample_memory_doc()).expect("save memory");

        let updated = update_document(
            &working_dir,
            KnowledgeUpdateRequest {
                op: KnowledgeUpdateOp::UpdateBody,
                path: MEMORY_USER_PREFERENCE_PATH.to_string(),
                doc_type: Some(KnowledgeType::Memory),
                body: Some(Some("# 输出方式\n## 细节\n- 结论优先".to_string())),
                ..Default::default()
            },
        )
        .expect("update memory");

        assert_eq!(updated.body, "# 输出方式\n## 细节\n- 结论优先");

        let raw = std::fs::read_to_string(
            document_path(
                &working_dir,
                KnowledgeType::Memory,
                MEMORY_USER_PREFERENCE_PATH,
            )
            .expect("path"),
        )
        .expect("read memory");
        assert!(raw.contains(MEMORY_BODY_START));
        assert!(raw.contains(MEMORY_BODY_END));
        assert!(raw.contains("# 输出方式\n## 细节\n- 结论优先"));
        assert!(!raw.contains("## Content"));
    }

    #[test]
    fn save_document_preserves_disabled_summary_and_rules_in_frontmatter_cache() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let mut doc = sample_doc();
        doc.summary_enabled = false;
        doc.explicit_maintenance_rules = false;
        doc.maintenance_rules = Some("Keep only durable notes".to_string());
        save_document(&working_dir, doc).expect("save");

        let raw = std::fs::read_to_string(
            document_path(&working_dir, KnowledgeType::Design, "gameplay/core-loop.md").unwrap(),
        )
        .expect("read raw");
        assert!(raw.contains("summaryCache: Short summary"));
        assert!(raw.contains("maintenanceRulesCache: Keep only durable notes"));
        assert!(!raw.contains("## Summary"));
        assert!(!raw.contains("## Maintenance Rules"));

        let reread =
            load_document_by_path(&working_dir, KnowledgeType::Design, "gameplay/core-loop.md")
                .expect("reload");
        assert!(!reread.summary_enabled);
        assert_eq!(reread.summary.as_deref(), Some("Short summary"));
        assert!(!reread.explicit_maintenance_rules);
        assert_eq!(
            reread.maintenance_rules.as_deref(),
            Some("Keep only durable notes")
        );
    }

    #[test]
    fn update_meta_preserves_cached_summary_and_rules_when_switches_turn_off() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let mut doc = sample_doc();
        doc.explicit_maintenance_rules = true;
        doc.maintenance_rules = Some("Keep only durable notes".to_string());
        save_document(&working_dir, doc).expect("save");

        let updated = update_document(
            &working_dir,
            KnowledgeUpdateRequest {
                op: KnowledgeUpdateOp::UpdateMeta,
                path: "gameplay/core-loop.md".to_string(),
                doc_type: Some(KnowledgeType::Design),
                summary_enabled: Some(false),
                explicit_maintenance_rules: Some(false),
                ..Default::default()
            },
        )
        .expect("update meta");

        assert!(!updated.summary_enabled);
        assert_eq!(updated.summary.as_deref(), Some("Short summary"));
        assert!(!updated.explicit_maintenance_rules);
        assert_eq!(
            updated.maintenance_rules.as_deref(),
            Some("Keep only durable notes")
        );

        let reread =
            load_document_by_path(&working_dir, KnowledgeType::Design, "gameplay/core-loop.md")
                .expect("reload");
        assert!(!reread.summary_enabled);
        assert_eq!(reread.summary.as_deref(), Some("Short summary"));
        assert!(!reread.explicit_maintenance_rules);
        assert_eq!(
            reread.maintenance_rules.as_deref(),
            Some("Keep only durable notes")
        );
    }

    #[test]
    fn create_memory_document_defaults_to_auto_with_rules() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let created = update_document(
            &working_dir,
            KnowledgeUpdateRequest {
                op: KnowledgeUpdateOp::Create,
                path: "project-understanding.md".to_string(),
                doc_type: Some(KnowledgeType::Memory),
                title: Some("Project Understanding".to_string()),
                body: Some(Some(String::new())),
                ..Default::default()
            },
        )
        .expect("create");

        assert!(created.ai_maintained);
        assert_eq!(created.read_only, false);
        assert!(!created.summary_enabled);
        assert!(created
            .maintenance_rules
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false));
    }

    #[test]
    fn directory_config_uses_sibling_locus_meta_path() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        std::fs::create_dir_all(
            knowledge_root(&working_dir)
                .join("memory")
                .join("project-structure"),
        )
        .unwrap();

        let saved = update_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            "project-structure",
            sample_directory_config(),
        )
        .expect("save config");

        assert_eq!(saved.config_path, "project-structure.locus-meta");
        assert!(knowledge_root(&working_dir)
            .join("memory")
            .join("project-structure.locus-meta")
            .is_file());
        let raw = std::fs::read_to_string(
            knowledge_root(&working_dir)
                .join("memory")
                .join("project-structure.locus-meta"),
        )
        .expect("read config");
        assert!(raw.contains("version: 4"));
        assert!(raw.contains("injectMode: excerpt"));
        assert!(raw.contains("aiMaintained:"));
        assert!(raw.contains("explicitMaintenanceRules:"));
        assert!(raw.contains("lexicalSearch: enabled"));
        assert!(raw.contains("vectorSearch: disabled"));
        assert!(!raw.contains("externalSources:"));
    }

    #[test]
    fn reference_directory_external_sources_are_preserved_across_config_updates() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        std::fs::create_dir_all(
            knowledge_root(&working_dir)
                .join("reference")
                .join("feishu-knowledge-base"),
        )
        .unwrap();

        update_directory_config(
            &working_dir,
            KnowledgeType::Reference,
            "feishu-knowledge-base",
            sample_directory_config(),
        )
        .expect("save reference config");
        update_directory_external_sources(
            &working_dir,
            KnowledgeType::Reference,
            "feishu-knowledge-base",
            vec![KnowledgeExternalSource {
                provider: KnowledgeSourceProvider::Feishu,
                locator: Some("space:test-space;node:test-node".to_string()),
                source_id: Some("test-node".to_string()),
                sync_enabled: true,
            }],
        )
        .expect("save external sources");

        let mut updated = sample_directory_config();
        updated.summary = "Updated summary".to_string();
        let record = update_directory_config(
            &working_dir,
            KnowledgeType::Reference,
            "feishu-knowledge-base",
            updated,
        )
        .expect("update config");

        assert_eq!(record.config.summary, "Updated summary");
        assert_eq!(record.external_sources.len(), 1);
        assert_eq!(
            record.external_sources[0].provider,
            KnowledgeSourceProvider::Feishu
        );
        assert_eq!(
            record.external_sources[0].source_id.as_deref(),
            Some("test-node")
        );

        let raw = std::fs::read_to_string(
            knowledge_root(&working_dir)
                .join("reference")
                .join("feishu-knowledge-base.locus-meta"),
        )
        .expect("read reference config");
        assert!(raw.contains("externalSources:"));
    }

    #[test]
    fn directory_external_sources_are_restricted_to_reference() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        for (doc_type, path) in [
            (KnowledgeType::Design, "combat"),
            (KnowledgeType::Memory, "project-structure"),
            (KnowledgeType::Skill, "workflows"),
        ] {
            std::fs::create_dir_all(type_root(&working_dir, doc_type).join(path))
                .expect("create directory");

            update_directory_config(&working_dir, doc_type, path, sample_directory_config())
                .expect("save config");

            let error = update_directory_external_sources(
                &working_dir,
                doc_type,
                path,
                vec![KnowledgeExternalSource {
                    provider: KnowledgeSourceProvider::Feishu,
                    locator: Some("space:test-space;node:test-node".to_string()),
                    source_id: Some("test-node".to_string()),
                    sync_enabled: true,
                }],
            )
            .expect_err("non-reference directories reject external sources");

            assert_eq!(
                error,
                "Directory external sources are only supported for reference"
            );
        }
    }

    #[test]
    fn find_reference_directory_by_external_provider_returns_matching_directory() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        for path in ["unity-official-docs", "feishu-knowledge-base"] {
            std::fs::create_dir_all(knowledge_root(&working_dir).join("reference").join(path))
                .expect("create reference directory");
            update_directory_config(
                &working_dir,
                KnowledgeType::Reference,
                path,
                sample_directory_config(),
            )
            .expect("save reference config");
        }

        update_directory_external_sources(
            &working_dir,
            KnowledgeType::Reference,
            "unity-official-docs",
            vec![KnowledgeExternalSource {
                provider: KnowledgeSourceProvider::Unity,
                locator: Some("project:2022.3.47f1;docs:2022.3;locale:zh-CN".to_string()),
                source_id: Some("2022.3".to_string()),
                sync_enabled: true,
            }],
        )
        .expect("bind unity source");
        update_directory_external_sources(
            &working_dir,
            KnowledgeType::Reference,
            "feishu-knowledge-base",
            vec![KnowledgeExternalSource {
                provider: KnowledgeSourceProvider::Feishu,
                locator: Some("space:test-space".to_string()),
                source_id: Some("test-space".to_string()),
                sync_enabled: true,
            }],
        )
        .expect("bind feishu source");

        let matched = find_reference_directory_by_external_provider(
            &working_dir,
            KnowledgeSourceProvider::Unity,
        )
        .expect("find unity directory")
        .expect("unity directory should exist");

        assert_eq!(matched.path, "unity-official-docs");
        assert_eq!(matched.external_sources.len(), 1);
        assert_eq!(
            matched.external_sources[0].provider,
            KnowledgeSourceProvider::Unity
        );
    }

    #[test]
    fn list_reference_external_directory_bindings_returns_only_bound_directories() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        for path in ["example-reference", "unity-official-docs", "notes"] {
            std::fs::create_dir_all(knowledge_root(&working_dir).join("reference").join(path))
                .expect("create reference directory");
            update_directory_config(
                &working_dir,
                KnowledgeType::Reference,
                path,
                sample_directory_config(),
            )
            .expect("save reference config");
        }

        update_directory_external_sources(
            &working_dir,
            KnowledgeType::Reference,
            "example-reference",
            vec![KnowledgeExternalSource {
                provider: KnowledgeSourceProvider::LocalFolder,
                locator: Some("file:///F:/Docs/ExampleProjectReference".to_string()),
                source_id: Some("example-reference".to_string()),
                sync_enabled: false,
            }],
        )
        .expect("bind local folder source");
        update_directory_external_sources(
            &working_dir,
            KnowledgeType::Reference,
            "unity-official-docs",
            vec![KnowledgeExternalSource {
                provider: KnowledgeSourceProvider::Unity,
                locator: Some("project:2022.3.47f1;docs:2022.3;locale:zh-CN".to_string()),
                source_id: Some("unity-2022.3".to_string()),
                sync_enabled: true,
            }],
        )
        .expect("bind unity source");

        let bindings =
            list_reference_external_directory_bindings(&working_dir).expect("list bindings");

        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].path, "example-reference");
        assert_eq!(
            bindings[0].external_sources[0].provider,
            KnowledgeSourceProvider::LocalFolder
        );
        assert_eq!(bindings[1].path, "unity-official-docs");
        assert_eq!(
            bindings[1].external_sources[0].provider,
            KnowledgeSourceProvider::Unity
        );
    }

    #[test]
    fn delete_external_reference_directory_allows_local_folder_read_only_documents() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        std::fs::create_dir_all(
            knowledge_root(&working_dir)
                .join("reference")
                .join("example-reference")
                .join("gameplay"),
        )
        .expect("create external reference directory");
        update_directory_config(
            &working_dir,
            KnowledgeType::Reference,
            "example-reference",
            sample_directory_config(),
        )
        .expect("save reference config");
        update_directory_external_sources(
            &working_dir,
            KnowledgeType::Reference,
            "example-reference",
            vec![KnowledgeExternalSource {
                provider: KnowledgeSourceProvider::LocalFolder,
                locator: Some("file:///F:/Docs/ExampleProjectReference".to_string()),
                source_id: Some("example-reference".to_string()),
                sync_enabled: false,
            }],
        )
        .expect("bind local folder source");

        let mut doc = sample_doc();
        doc.doc_type = KnowledgeType::Reference;
        doc.path = "example-reference/gameplay/Gameplay.md".to_string();
        doc.title = "Gameplay".to_string();
        doc.read_only = true;
        doc.external_source = Some(KnowledgeExternalSource {
            provider: KnowledgeSourceProvider::LocalFolder,
            locator: Some("file:///F:/Docs/ExampleProjectReference".to_string()),
            source_id: Some("example-reference".to_string()),
            sync_enabled: false,
        });
        save_document(&working_dir, doc).expect("save external doc");

        let regular_delete_error =
            delete_directory(&working_dir, KnowledgeType::Reference, "example-reference")
                .expect_err("regular delete should reject read-only external docs");
        assert!(regular_delete_error.contains("read-only"));

        delete_external_reference_directory(&working_dir, "example-reference")
            .expect("delete external directory");

        assert!(!knowledge_root(&working_dir)
            .join("reference")
            .join("example-reference")
            .exists());
        assert!(!knowledge_root(&working_dir)
            .join("reference")
            .join("example-reference.locus-meta")
            .exists());
    }

    #[test]
    fn read_directory_config_defaults_legacy_inject_mode_to_excerpt() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let root = knowledge_root(&working_dir).join("design");
        std::fs::create_dir_all(root.join("combat")).unwrap();
        std::fs::write(
            root.join("combat.meta"),
            "version: 2\nsummary: legacy summary\naiMaintained: false\nexplicitMaintenanceRules: false\ninheritToChildren: true\nallowCreateDocuments: true\nallowCreateDirectories: true\nallowMoveDocuments: true\nallowMoveDirectories: true\nmaintenanceRules: \"\"\n",
        )
        .unwrap();

        let config = read_directory_config(&working_dir, KnowledgeType::Design, "combat")
            .expect("read legacy config");
        assert_eq!(config.config.version, 4);
        assert_eq!(config.config.inject_mode, KnowledgeInjectMode::Excerpt);
        assert_eq!(config.config.summary, "legacy summary");
        assert_eq!(config.config_path, "combat.locus-meta");
        assert!(!root.join("combat.meta").exists());
        assert!(root.join("combat.locus-meta").is_file());
    }

    #[test]
    fn read_directory_config_inherits_direct_parent_rules_when_missing_local_meta() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let root = knowledge_root(&working_dir).join("design");
        std::fs::create_dir_all(root.join("combat").join("notes")).unwrap();

        let mut parent = sample_directory_config();
        parent.summary = "父目录摘要".to_string();
        parent.inject_mode = KnowledgeInjectMode::Path;
        parent.maintenance_rules = "- Inherit stable combat structure rules".to_string();
        update_directory_config(&working_dir, KnowledgeType::Design, "combat", parent)
            .expect("save parent config");

        let config = read_directory_config(&working_dir, KnowledgeType::Design, "combat/notes")
            .expect("read child config");
        assert!(!config.exists);
        assert_eq!(config.config_path, "combat/notes.locus-meta");
        assert_eq!(config.config.summary, "");
        assert_eq!(config.config.inject_mode, KnowledgeInjectMode::Path);
        assert!(config.config.inherit_inject_mode);
        assert!(config.config.ai_maintained);
        assert!(config.config.inherit_ai_config);
        assert!(config.config.explicit_maintenance_rules);
        assert_eq!(
            config.config.lexical_search,
            FolderIndexRuleSetting::Inherit
        );
        assert_eq!(config.config.vector_search, FolderIndexRuleSetting::Inherit);
        assert_eq!(
            config.config.maintenance_rules,
            "- Inherit stable combat structure rules"
        );
        assert!(config.effective_lexical_search.enabled);
        assert_eq!(config.effective_lexical_search.source, "parent");
        assert_eq!(
            config.effective_lexical_search.source_dir.as_deref(),
            Some("combat")
        );
        assert!(!config.effective_vector_search.enabled);
        assert_eq!(config.effective_vector_search.source, "parent");
        assert_eq!(
            config.effective_vector_search.source_dir.as_deref(),
            Some("combat")
        );
        assert_eq!(
            config.inject_mode_source.kind,
            KnowledgeConfigSourceKind::ParentDirectory
        );
        assert_eq!(config.inject_mode_source.path.as_deref(), Some("combat"));
        assert_eq!(
            config.ai_config_source.kind,
            KnowledgeConfigSourceKind::ParentDirectory
        );
        assert_eq!(config.ai_config_source.path.as_deref(), Some("combat"));
    }

    #[test]
    fn read_directory_config_falls_back_when_parent_disables_inheritance() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let root = knowledge_root(&working_dir).join("design");
        std::fs::create_dir_all(root.join("combat").join("notes")).unwrap();

        let mut parent = sample_directory_config();
        parent.inherit_to_children = false;
        update_directory_config(&working_dir, KnowledgeType::Design, "combat", parent)
            .expect("save parent config");

        let config = read_directory_config(&working_dir, KnowledgeType::Design, "combat/notes")
            .expect("read child config");
        assert!(!config.exists);
        assert_eq!(config.config.summary, "");
        assert_eq!(config.config.inject_mode, KnowledgeInjectMode::Excerpt);
        assert!(config.config.inherit_inject_mode);
        assert!(!config.config.ai_maintained);
        assert!(config.config.inherit_ai_config);
        assert!(!config.config.explicit_maintenance_rules);
        assert_eq!(
            config.config.lexical_search,
            FolderIndexRuleSetting::Inherit
        );
        assert_eq!(config.config.vector_search, FolderIndexRuleSetting::Inherit);
        assert!(config.config.maintenance_rules.is_empty());
        assert!(config.effective_lexical_search.enabled);
        assert_eq!(config.effective_lexical_search.source, "default");
        assert!(config.effective_lexical_search.source_dir.is_none());
        assert!(config.effective_vector_search.enabled);
        assert_eq!(config.effective_vector_search.source, "default");
        assert!(config.effective_vector_search.source_dir.is_none());
        assert_eq!(
            config.inject_mode_source.kind,
            KnowledgeConfigSourceKind::TypeDefault
        );
        assert!(config.inject_mode_source.path.is_none());
        assert_eq!(
            config.ai_config_source.kind,
            KnowledgeConfigSourceKind::TypeDefault
        );
        assert!(config.ai_config_source.path.is_none());
    }

    #[test]
    fn default_document_create_patch_uses_memory_root_defaults() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let patch = default_document_create_patch(
            &working_dir,
            KnowledgeType::Memory,
            "project-context.md",
        )
        .expect("build patch");

        assert_eq!(patch.title.as_deref(), Some("project-context"));
        assert_eq!(patch.body, Some(Some(String::new())));
        assert_eq!(patch.inherit_inject_mode, Some(true));
        assert_eq!(patch.inherit_ai_config, Some(true));
        assert!(patch.ai_maintained.is_none());
        assert!(patch.explicit_maintenance_rules.is_none());
        assert!(patch.maintenance_rules.is_none());
    }

    #[test]
    fn default_document_create_patch_inherits_direct_parent_rules() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let root = knowledge_root(&working_dir).join("design");
        std::fs::create_dir_all(root.join("combat")).unwrap();

        let mut parent = sample_directory_config();
        parent.maintenance_rules =
            "- Record combat child docs with verified constraints".to_string();
        update_directory_config(&working_dir, KnowledgeType::Design, "combat", parent)
            .expect("save parent config");

        let patch = default_document_create_patch(
            &working_dir,
            KnowledgeType::Design,
            "combat/core-loop.md",
        )
        .expect("build patch");

        assert_eq!(patch.title.as_deref(), Some("core-loop"));
        assert_eq!(patch.inherit_inject_mode, Some(true));
        assert_eq!(patch.inherit_ai_config, Some(true));
        assert!(patch.ai_maintained.is_none());
        assert!(patch.explicit_maintenance_rules.is_none());
        assert!(patch.maintenance_rules.is_none());
    }

    #[test]
    fn ensure_memory_builtin_documents_seeds_builtin_memory_defaults() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        ensure_memory_builtin_documents(&working_dir).expect("seed builtins");

        let docs = list_documents(&working_dir, Some(KnowledgeType::Memory), None).expect("list");
        let paths = docs.iter().map(|doc| doc.path.as_str()).collect::<Vec<_>>();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&MEMORY_PROJECT_MISTAKE_NOTE_PATH));
        assert!(paths.contains(&MEMORY_USER_PREFERENCE_PATH));
        assert!(!paths.contains(&"project-understanding.md"));

        let user_pref = load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_USER_PREFERENCE_PATH,
        )
        .expect("load user preference");
        assert_eq!(user_pref.inject_mode, KnowledgeInjectMode::Rule);
        assert!(user_pref.ai_maintained);
        assert!(user_pref.body.is_empty());

        let directories =
            list_directories(&working_dir, KnowledgeType::Memory).expect("list directories");
        assert!(directories.contains(&"unity-project-understanding".to_string()));

        let directory = read_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            "unity-project-understanding",
        )
        .expect("read builtin memory directory");
        assert!(directory.exists);
        assert_eq!(directory.config.inject_mode, KnowledgeInjectMode::Path);
        assert!(directory.config.ai_maintained);
        assert!(directory.config.explicit_maintenance_rules);
        assert!(directory.config.summary.contains("Unity"));
        assert!(directory
            .config
            .maintenance_rules
            .contains("Write user-supplied design goals"));
        assert!(directory
            .config
            .maintenance_rules
            .contains("Maintain only project-derived engineering understanding"));
    }

    #[test]
    fn ensure_memory_builtin_documents_updates_builtin_directory_rules_for_seed_v4() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        create_directory(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("create builtin memory directory");
        update_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
            KnowledgeDirectoryConfig {
                version: default_directory_config_version(),
                summary: MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY.to_string(),
                inject_mode: KnowledgeInjectMode::Excerpt,
                inherit_inject_mode: false,
                ai_maintained: true,
                inherit_ai_config: false,
                explicit_maintenance_rules: true,
                lexical_search: FolderIndexRuleSetting::Inherit,
                vector_search: FolderIndexRuleSetting::Inherit,
                inherit_to_children: true,
                allow_create_documents: true,
                allow_create_directories: true,
                allow_move_documents: true,
                allow_move_directories: true,
                maintenance_rules: MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES_V4.to_string(),
            },
        )
        .expect("seed old builtin directory rules");
        write_memory_builtin_seed_version(&working_dir, 4).expect("write old seed version");

        ensure_memory_builtin_documents(&working_dir).expect("upgrade builtins");

        let directory = read_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("read upgraded builtin directory");
        assert_eq!(
            directory.config.maintenance_rules,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES
        );
        assert_eq!(directory.config.inject_mode, KnowledgeInjectMode::Path);
    }

    #[test]
    fn ensure_memory_builtin_documents_preserves_custom_memory_directory_rules() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        create_directory(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("create builtin memory directory");
        update_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
            KnowledgeDirectoryConfig {
                version: default_directory_config_version(),
                summary: MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY.to_string(),
                inject_mode: KnowledgeInjectMode::Excerpt,
                inherit_inject_mode: false,
                ai_maintained: true,
                inherit_ai_config: false,
                explicit_maintenance_rules: true,
                lexical_search: FolderIndexRuleSetting::Inherit,
                vector_search: FolderIndexRuleSetting::Inherit,
                inherit_to_children: true,
                allow_create_documents: true,
                allow_create_directories: true,
                allow_move_documents: true,
                allow_move_directories: true,
                maintenance_rules: "- 自定义规则".to_string(),
            },
        )
        .expect("seed custom builtin directory rules");
        write_memory_builtin_seed_version(&working_dir, 4).expect("write old seed version");

        ensure_memory_builtin_documents(&working_dir).expect("upgrade builtins");

        let directory = read_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("read upgraded builtin directory");
        assert_eq!(directory.config.maintenance_rules, "- 自定义规则");
        assert_eq!(directory.config.inject_mode, KnowledgeInjectMode::Excerpt);
    }

    #[test]
    fn ensure_memory_builtin_documents_updates_builtin_directory_inject_mode_for_seed_v5() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        create_directory(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("create builtin memory directory");
        update_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
            KnowledgeDirectoryConfig {
                version: default_directory_config_version(),
                summary: MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY.to_string(),
                inject_mode: KnowledgeInjectMode::Excerpt,
                inherit_inject_mode: false,
                ai_maintained: true,
                inherit_ai_config: false,
                explicit_maintenance_rules: true,
                lexical_search: FolderIndexRuleSetting::Inherit,
                vector_search: FolderIndexRuleSetting::Inherit,
                inherit_to_children: true,
                allow_create_documents: true,
                allow_create_directories: true,
                allow_move_documents: true,
                allow_move_directories: true,
                maintenance_rules: MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES.to_string(),
            },
        )
        .expect("seed old builtin directory inject mode");
        write_memory_builtin_seed_version(&working_dir, 5).expect("write old seed version");

        ensure_memory_builtin_documents(&working_dir).expect("upgrade builtins");

        let directory = read_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("read upgraded builtin directory");
        assert_eq!(directory.config.inject_mode, KnowledgeInjectMode::Path);
    }

    #[test]
    fn ensure_memory_builtin_documents_updates_builtin_doc_rules_for_seed_v6() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        for (id, path, title, rules) in [
            (
                "kd_builtin_memory_project_mistake_note",
                MEMORY_PROJECT_MISTAKE_NOTE_LEGACY_PATH,
                "错题本",
                MEMORY_PROJECT_MISTAKE_NOTE_RULES_V6,
            ),
            (
                "kd_builtin_memory_user_preference",
                MEMORY_USER_PREFERENCE_LEGACY_PATH,
                "用户偏好",
                MEMORY_USER_PREFERENCE_RULES_V6,
            ),
        ] {
            save_document(
                &working_dir,
                KnowledgeDocument {
                    id: id.to_string(),
                    doc_type: KnowledgeType::Memory,
                    path: path.to_string(),
                    title: title.to_string(),
                    inject_mode: KnowledgeInjectMode::Full,
                    inherit_inject_mode: false,
                    inject_mode_source: self_config_source(),
                    summary_enabled: false,
                    command_enabled: false,
                    read_only: false,
                    ai_maintained: true,
                    storage_source: KnowledgeStorageSource::Project,
                    inherit_ai_config: false,
                    ai_config_source: self_config_source(),
                    explicit_maintenance_rules: true,
                    external_source: None,
                    skill_enabled: None,
                    skill_surface: None,
                    command_trigger: None,
                    argument_hint: None,
                    tools: Vec::new(),
                    summary: None,
                    body: String::new(),
                    maintenance_rules: Some(rules.to_string()),
                    created_at: 1,
                    updated_at: 1,
                },
            )
            .expect("seed old builtin doc");
        }
        write_memory_builtin_seed_version(&working_dir, 6).expect("write old seed version");

        ensure_memory_builtin_documents(&working_dir).expect("upgrade builtins");

        let mistake_note = load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_PROJECT_MISTAKE_NOTE_PATH,
        )
        .expect("load mistake note");
        assert_eq!(mistake_note.inject_mode, KnowledgeInjectMode::Full);
        assert_eq!(
            mistake_note.maintenance_rules.as_deref(),
            Some(MEMORY_PROJECT_MISTAKE_NOTE_RULES)
        );

        let user_preference = load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_USER_PREFERENCE_PATH,
        )
        .expect("load user preference");
        assert_eq!(user_preference.inject_mode, KnowledgeInjectMode::Rule);
        assert_eq!(
            user_preference.maintenance_rules.as_deref(),
            Some(MEMORY_USER_PREFERENCE_RULES)
        );
    }

    #[test]
    fn ensure_memory_builtin_documents_preserves_custom_builtin_doc_rules() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_builtin_memory_project_mistake_note".to_string(),
                doc_type: KnowledgeType::Memory,
                path: MEMORY_PROJECT_MISTAKE_NOTE_PATH.to_string(),
                title: "错题本".to_string(),
                inject_mode: KnowledgeInjectMode::Full,
                inherit_inject_mode: false,
                inject_mode_source: self_config_source(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: true,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: self_config_source(),
                explicit_maintenance_rules: true,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                tools: Vec::new(),
                summary: None,
                body: String::new(),
                maintenance_rules: Some("- 自定义错题规则".to_string()),
                created_at: 1,
                updated_at: 1,
            },
        )
        .expect("save custom builtin doc");
        write_memory_builtin_seed_version(&working_dir, 6).expect("write old seed version");

        ensure_memory_builtin_documents(&working_dir).expect("upgrade builtins");

        let mistake_note = load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_PROJECT_MISTAKE_NOTE_PATH,
        )
        .expect("load mistake note");
        assert_eq!(
            mistake_note.maintenance_rules.as_deref(),
            Some("- 自定义错题规则")
        );
    }

    #[test]
    fn ensure_memory_builtin_documents_renames_legacy_builtin_paths() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_builtin_memory_user_preference".to_string(),
                doc_type: KnowledgeType::Memory,
                path: MEMORY_USER_PREFERENCE_LEGACY_PATH.to_string(),
                title: "用户偏好".to_string(),
                inject_mode: KnowledgeInjectMode::Full,
                inherit_inject_mode: false,
                inject_mode_source: self_config_source(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: true,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: self_config_source(),
                explicit_maintenance_rules: true,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                tools: Vec::new(),
                summary: None,
                body: "- 统一使用中文".to_string(),
                maintenance_rules: Some(MEMORY_USER_PREFERENCE_RULES.to_string()),
                created_at: 1,
                updated_at: 1,
            },
        )
        .expect("save legacy builtin doc");

        ensure_memory_builtin_documents(&working_dir).expect("upgrade builtins");

        assert!(!document_path(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_USER_PREFERENCE_LEGACY_PATH
        )
        .expect("legacy path")
        .is_file());

        let migrated = load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_USER_PREFERENCE_PATH,
        )
        .expect("load renamed builtin doc");
        assert_eq!(migrated.body, "- 统一使用中文");
        assert_eq!(migrated.inject_mode, KnowledgeInjectMode::Rule);
    }

    #[test]
    fn ensure_memory_builtin_documents_promotes_user_preference_to_rule_in_seed_v8() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_builtin_memory_user_preference".to_string(),
                doc_type: KnowledgeType::Memory,
                path: MEMORY_USER_PREFERENCE_PATH.to_string(),
                title: "用户偏好".to_string(),
                inject_mode: KnowledgeInjectMode::Full,
                inherit_inject_mode: false,
                inject_mode_source: self_config_source(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: true,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: self_config_source(),
                explicit_maintenance_rules: true,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                tools: Vec::new(),
                summary: None,
                body: "- 汇报先给结论".to_string(),
                maintenance_rules: Some(MEMORY_USER_PREFERENCE_RULES.to_string()),
                created_at: 1,
                updated_at: 1,
            },
        )
        .expect("save v8 builtin doc");
        write_memory_builtin_seed_version(&working_dir, 8).expect("write old seed version");

        ensure_memory_builtin_documents(&working_dir).expect("upgrade builtins");

        let migrated = load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_USER_PREFERENCE_PATH,
        )
        .expect("load upgraded user preference");
        assert_eq!(migrated.inject_mode, KnowledgeInjectMode::Rule);
        assert_eq!(migrated.body, "- 汇报先给结论");
    }

    #[test]
    fn ensure_memory_builtin_documents_updates_builtin_rules_for_seed_v9() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        create_directory(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("create builtin memory directory");
        update_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
            KnowledgeDirectoryConfig {
                version: default_directory_config_version(),
                summary: MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY.to_string(),
                inject_mode: KnowledgeInjectMode::Path,
                inherit_inject_mode: false,
                ai_maintained: true,
                inherit_ai_config: false,
                explicit_maintenance_rules: true,
                lexical_search: FolderIndexRuleSetting::Inherit,
                vector_search: FolderIndexRuleSetting::Inherit,
                inherit_to_children: true,
                allow_create_documents: true,
                allow_create_directories: true,
                allow_move_documents: true,
                allow_move_directories: true,
                maintenance_rules: MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES_V9.to_string(),
            },
        )
        .expect("seed v9 builtin directory rules");

        for (id, path, title, inject_mode, rules) in [
            (
                "kd_builtin_memory_project_mistake_note",
                MEMORY_PROJECT_MISTAKE_NOTE_PATH,
                "错题本",
                KnowledgeInjectMode::Full,
                MEMORY_PROJECT_MISTAKE_NOTE_RULES_V9,
            ),
            (
                "kd_builtin_memory_user_preference",
                MEMORY_USER_PREFERENCE_PATH,
                "用户偏好",
                KnowledgeInjectMode::Rule,
                MEMORY_USER_PREFERENCE_RULES_V9,
            ),
        ] {
            save_document(
                &working_dir,
                KnowledgeDocument {
                    id: id.to_string(),
                    doc_type: KnowledgeType::Memory,
                    path: path.to_string(),
                    title: title.to_string(),
                    inject_mode,
                    inherit_inject_mode: false,
                    inject_mode_source: self_config_source(),
                    summary_enabled: false,
                    command_enabled: false,
                    read_only: false,
                    ai_maintained: true,
                    storage_source: KnowledgeStorageSource::Project,
                    inherit_ai_config: false,
                    ai_config_source: self_config_source(),
                    explicit_maintenance_rules: true,
                    external_source: None,
                    skill_enabled: None,
                    skill_surface: None,
                    command_trigger: None,
                    argument_hint: None,
                    tools: Vec::new(),
                    summary: None,
                    body: String::new(),
                    maintenance_rules: Some(rules.to_string()),
                    created_at: 1,
                    updated_at: 1,
                },
            )
            .expect("seed v9 builtin doc");
        }
        write_memory_builtin_seed_version(&working_dir, 9).expect("write old seed version");

        ensure_memory_builtin_documents(&working_dir).expect("upgrade builtins");

        let directory = read_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("read upgraded builtin directory");
        assert_eq!(
            directory.config.maintenance_rules,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES
        );

        let mistake_note = load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_PROJECT_MISTAKE_NOTE_PATH,
        )
        .expect("load upgraded mistake note");
        assert_eq!(
            mistake_note.maintenance_rules.as_deref(),
            Some(MEMORY_PROJECT_MISTAKE_NOTE_RULES)
        );

        let user_preference = load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_USER_PREFERENCE_PATH,
        )
        .expect("load upgraded user preference");
        assert_eq!(
            user_preference.maintenance_rules.as_deref(),
            Some(MEMORY_USER_PREFERENCE_RULES)
        );
    }

    #[test]
    fn ensure_memory_builtin_documents_updates_builtin_summary_for_seed_v10() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        create_directory(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("create builtin memory directory");
        update_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
            KnowledgeDirectoryConfig {
                version: default_directory_config_version(),
                summary: MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY_V10.to_string(),
                inject_mode: KnowledgeInjectMode::Path,
                inherit_inject_mode: false,
                ai_maintained: true,
                inherit_ai_config: false,
                explicit_maintenance_rules: true,
                lexical_search: FolderIndexRuleSetting::Inherit,
                vector_search: FolderIndexRuleSetting::Inherit,
                inherit_to_children: true,
                allow_create_documents: true,
                allow_create_directories: true,
                allow_move_documents: true,
                allow_move_directories: true,
                maintenance_rules: MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES.to_string(),
            },
        )
        .expect("seed v10 builtin directory summary");
        write_memory_builtin_seed_version(&working_dir, 10).expect("write old seed version");

        ensure_memory_builtin_documents(&working_dir).expect("upgrade builtins");

        let directory = read_directory_config(
            &working_dir,
            KnowledgeType::Memory,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_PATH,
        )
        .expect("read upgraded builtin directory");
        assert_eq!(
            directory.config.summary,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_SUMMARY
        );
        assert_eq!(
            directory.config.maintenance_rules,
            MEMORY_UNITY_PROJECT_UNDERSTANDING_RULES
        );
    }

    #[test]
    fn ensure_memory_builtin_documents_migrates_legacy_project_understanding() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_custom_project_understanding".to_string(),
                doc_type: KnowledgeType::Memory,
                path: "project-understanding.md".to_string(),
                title: "自定义项目理解".to_string(),
                inject_mode: KnowledgeInjectMode::Full,
                inherit_inject_mode: false,
                inject_mode_source: self_config_source(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: true,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: self_config_source(),
                explicit_maintenance_rules: true,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                tools: Vec::new(),
                summary: None,
                body: "已有内容".to_string(),
                maintenance_rules: Some("已有规则".to_string()),
                created_at: 1,
                updated_at: 1,
            },
        )
        .expect("save existing");

        ensure_memory_builtin_documents(&working_dir).expect("seed builtins");

        assert!(load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            "project-understanding.md"
        )
        .is_err());

        let migrated = load_document_by_path(
            &working_dir,
            KnowledgeType::Memory,
            "unity-project-understanding/overview.md",
        )
        .expect("reload migrated");
        assert_eq!(migrated.id, "kd_custom_project_understanding");
        assert_eq!(migrated.title, "overview");
        assert_eq!(migrated.body, "已有内容");

        let docs = list_documents(&working_dir, Some(KnowledgeType::Memory), None).expect("list");
        assert_eq!(docs.len(), 3);
    }

    #[test]
    fn create_design_document_defaults_to_summary_disabled() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let created = update_document(
            &working_dir,
            KnowledgeUpdateRequest {
                op: KnowledgeUpdateOp::Create,
                path: "gameplay/core-loop.md".to_string(),
                doc_type: Some(KnowledgeType::Design),
                title: Some("Core Loop".to_string()),
                body: Some(Some(String::new())),
                ..Default::default()
            },
        )
        .expect("create");

        assert_eq!(created.inject_mode, KnowledgeInjectMode::Path);
        assert!(!created.summary_enabled);
        assert!(created.summary.is_none());
        assert!(!created.explicit_maintenance_rules);
        assert!(created.maintenance_rules.is_none());
    }

    #[test]
    fn update_meta_can_unlock_local_read_only_document() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let mut doc = sample_doc();
        doc.read_only = true;
        save_document(&working_dir, doc).expect("save");

        let updated = update_document(
            &working_dir,
            KnowledgeUpdateRequest {
                op: KnowledgeUpdateOp::UpdateMeta,
                path: "gameplay/core-loop.md".to_string(),
                doc_type: Some(KnowledgeType::Design),
                read_only: Some(false),
                ..Default::default()
            },
        )
        .expect("unlock");

        assert!(!updated.read_only);
    }

    #[test]
    fn edit_document_syncs_default_title_when_file_name_changes() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let mut doc = sample_doc();
        doc.title = "core-loop".to_string();
        save_document(&working_dir, doc).expect("save");

        let updated = edit_document(
            &working_dir,
            "gameplay/core-loop.md",
            Some(KnowledgeType::Design),
            KnowledgeDocumentPatch {
                new_path: Some("gameplay/systems-loop.md".to_string()),
                ..Default::default()
            },
        )
        .expect("rename");

        assert_eq!(updated.path, "gameplay/systems-loop.md");
        assert_eq!(updated.title, "systems-loop");
    }

    #[test]
    fn update_meta_syncs_title_when_file_name_changes() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        save_document(&working_dir, sample_doc()).expect("save");

        let updated = update_document(
            &working_dir,
            KnowledgeUpdateRequest {
                op: KnowledgeUpdateOp::UpdateMeta,
                path: "gameplay/core-loop.md".to_string(),
                doc_type: Some(KnowledgeType::Design),
                new_path: Some("gameplay/systems-loop.md".to_string()),
                ..Default::default()
            },
        )
        .expect("rename");

        assert_eq!(updated.path, "gameplay/systems-loop.md");
        assert_eq!(updated.title, "systems-loop");
    }

    #[test]
    fn move_directory_updates_nested_document_paths() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        save_document(&working_dir, sample_doc()).expect("save");
        std::fs::create_dir_all(
            knowledge_root(&working_dir)
                .join("design")
                .join("gameplay")
                .join("notes"),
        )
        .unwrap();

        let moved = move_directory(
            &working_dir,
            KnowledgeType::Design,
            "gameplay",
            "systems/gameplay",
        )
        .expect("move directory");

        assert_eq!(moved, "systems/gameplay");
        assert!(!knowledge_root(&working_dir)
            .join("design")
            .join("gameplay")
            .exists());
        assert!(knowledge_root(&working_dir)
            .join("design")
            .join("systems")
            .join("gameplay")
            .join("notes")
            .is_dir());

        let reread = load_document_by_path(
            &working_dir,
            KnowledgeType::Design,
            "systems/gameplay/core-loop.md",
        )
        .expect("load moved doc");
        assert_eq!(reread.path, "systems/gameplay/core-loop.md");
    }

    #[test]
    fn move_directory_moves_sidecar_config_and_allows_nested_locus_meta_files() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        save_document(&working_dir, sample_doc()).expect("save");
        std::fs::create_dir_all(
            knowledge_root(&working_dir)
                .join("design")
                .join("gameplay")
                .join("notes"),
        )
        .unwrap();
        update_directory_config(
            &working_dir,
            KnowledgeType::Design,
            "gameplay",
            sample_directory_config(),
        )
        .expect("save root config");
        update_directory_config(
            &working_dir,
            KnowledgeType::Design,
            "gameplay/notes",
            sample_directory_config(),
        )
        .expect("save nested config");

        move_directory(
            &working_dir,
            KnowledgeType::Design,
            "gameplay",
            "systems/gameplay",
        )
        .expect("move directory");

        assert!(!knowledge_root(&working_dir)
            .join("design")
            .join("gameplay.locus-meta")
            .exists());
        assert!(knowledge_root(&working_dir)
            .join("design")
            .join("systems")
            .join("gameplay.locus-meta")
            .is_file());
        assert!(knowledge_root(&working_dir)
            .join("design")
            .join("systems")
            .join("gameplay")
            .join("notes.locus-meta")
            .is_file());
    }

    #[test]
    fn move_directory_rejects_descendant_target() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        save_document(&working_dir, sample_doc()).expect("save");

        let result = move_directory(
            &working_dir,
            KnowledgeType::Design,
            "gameplay",
            "gameplay/archive",
        );

        assert!(result.is_err());
    }

    #[test]
    fn delete_directory_removes_nested_documents_and_folders() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let mut root_doc = sample_doc();
        root_doc.path = "gameplay/core-loop.md".to_string();
        save_document(&working_dir, root_doc).expect("save root doc");

        let mut nested_doc = sample_doc();
        nested_doc.id = "kd_nested".to_string();
        nested_doc.path = "gameplay/notes/archive.md".to_string();
        nested_doc.title = "Archive".to_string();
        save_document(&working_dir, nested_doc).expect("save nested doc");

        let deleted =
            delete_directory(&working_dir, KnowledgeType::Design, "gameplay").expect("delete");

        assert_eq!(deleted, "gameplay");
        assert!(!knowledge_root(&working_dir)
            .join("design")
            .join("gameplay")
            .exists());
        let remaining = list_documents(&working_dir, Some(KnowledgeType::Design), None).unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn delete_directory_removes_sidecar_config_files() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let mut doc = sample_doc();
        doc.path = "gameplay/core-loop.md".to_string();
        save_document(&working_dir, doc).expect("save");
        std::fs::create_dir_all(
            knowledge_root(&working_dir)
                .join("design")
                .join("gameplay")
                .join("notes"),
        )
        .unwrap();
        update_directory_config(
            &working_dir,
            KnowledgeType::Design,
            "gameplay",
            sample_directory_config(),
        )
        .expect("save root config");
        update_directory_config(
            &working_dir,
            KnowledgeType::Design,
            "gameplay/notes",
            sample_directory_config(),
        )
        .expect("save nested config");

        delete_directory(&working_dir, KnowledgeType::Design, "gameplay").expect("delete");

        assert!(!knowledge_root(&working_dir)
            .join("design")
            .join("gameplay.locus-meta")
            .exists());
        assert!(!knowledge_root(&working_dir)
            .join("design")
            .join("gameplay")
            .exists());
    }

    #[test]
    fn delete_directory_rejects_read_only_documents() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let mut doc = sample_doc();
        doc.read_only = true;
        save_document(&working_dir, doc).expect("save");

        let result = delete_directory(&working_dir, KnowledgeType::Design, "gameplay");

        assert!(result
            .expect_err("delete should fail")
            .contains("read-only"));
        assert!(knowledge_root(&working_dir)
            .join("design")
            .join("gameplay")
            .join("core-loop.md")
            .is_file());
    }

    #[test]
    fn delete_directory_tolerates_document_path_mismatch() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let file_path = knowledge_root(&working_dir)
            .join("reference")
            .join("unity-official-docs")
            .join("manual")
            .join("2d-introduction.md");
        let mut doc = sample_doc();
        doc.doc_type = KnowledgeType::Reference;
        doc.path = "manual/2d-introduction.md".to_string();
        doc.inject_mode = KnowledgeInjectMode::None;
        doc.summary_enabled = false;
        doc.command_enabled = false;
        doc.external_source = Some(KnowledgeExternalSource {
            provider: KnowledgeSourceProvider::Unity,
            locator: None,
            source_id: Some("unity-2022.3".to_string()),
            sync_enabled: true,
        });
        save_document_to_path(&file_path, doc).expect("save mismatched reference doc");

        let deleted = delete_directory(
            &working_dir,
            KnowledgeType::Reference,
            "unity-official-docs",
        )
        .expect("delete mismatched directory");

        assert_eq!(deleted, "unity-official-docs");
        assert!(!knowledge_root(&working_dir)
            .join("reference")
            .join("unity-official-docs")
            .exists());
    }

    #[test]
    fn delete_directory_still_rejects_mismatched_read_only_documents() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let file_path = knowledge_root(&working_dir)
            .join("reference")
            .join("example-reference")
            .join("manual")
            .join("gameplay.md");
        let mut doc = sample_doc();
        doc.doc_type = KnowledgeType::Reference;
        doc.path = "manual/gameplay.md".to_string();
        doc.inject_mode = KnowledgeInjectMode::None;
        doc.summary_enabled = false;
        doc.command_enabled = false;
        doc.external_source = Some(KnowledgeExternalSource {
            provider: KnowledgeSourceProvider::LocalFolder,
            locator: Some("file:///F:/Docs/ExampleProjectReference".to_string()),
            source_id: Some("example-reference".to_string()),
            sync_enabled: false,
        });
        save_document_to_path(&file_path, doc).expect("save mismatched read-only doc");

        let error = delete_directory(&working_dir, KnowledgeType::Reference, "example-reference")
            .expect_err("delete should still reject read-only doc");

        assert!(error.contains("read-only"));
        assert!(knowledge_root(&working_dir)
            .join("reference")
            .join("example-reference")
            .exists());
    }

    #[test]
    fn list_documents_with_app_root_prefers_workspace_over_app() {
        let workspace = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();
        let app = TempDir::new().unwrap();
        let app_root = app.path().join("knowledge");
        std::fs::create_dir_all(app_root.join("skill")).unwrap();

        let mut app_doc = sample_doc();
        app_doc.id = "kd_app_skill".to_string();
        app_doc.doc_type = KnowledgeType::Skill;
        app_doc.path = "shared.md".to_string();
        app_doc.title = "Shared".to_string();
        save_document_to_path(
            &document_path_in_root(&app_root, KnowledgeType::Skill, "shared.md").unwrap(),
            app_doc,
        )
        .expect("save app doc");

        let app_items = list_documents_with_app_root(
            &working_dir,
            Some(&app_root),
            Some(KnowledgeType::Skill),
            None,
        )
        .expect("list app docs");
        assert_eq!(app_items.len(), 1);
        assert_eq!(app_items[0].id, "kd_app_skill");
        assert!(app_items[0].read_only);
        assert_eq!(app_items[0].storage_source, KnowledgeStorageSource::App);

        let app_read = read_document_with_app_root(
            &working_dir,
            Some(&app_root),
            KnowledgeType::Skill,
            "shared.md",
            "full",
        )
        .expect("read app doc");
        assert_eq!(
            app_read.document.storage_source,
            KnowledgeStorageSource::App
        );

        let mut workspace_doc = sample_doc();
        workspace_doc.id = "kd_workspace_skill".to_string();
        workspace_doc.doc_type = KnowledgeType::Skill;
        workspace_doc.path = "shared.md".to_string();
        workspace_doc.title = "Shared".to_string();
        save_document(&working_dir, workspace_doc).expect("save workspace doc");

        let merged_items = list_documents_with_app_root(
            &working_dir,
            Some(&app_root),
            Some(KnowledgeType::Skill),
            None,
        )
        .expect("list merged docs");
        assert_eq!(merged_items.len(), 1);
        assert_eq!(merged_items[0].id, "kd_workspace_skill");
        assert!(!merged_items[0].read_only);
        assert_eq!(
            merged_items[0].storage_source,
            KnowledgeStorageSource::Project
        );

        let workspace_read = read_document_with_app_root(
            &working_dir,
            Some(&app_root),
            KnowledgeType::Skill,
            "shared.md",
            "full",
        )
        .expect("read workspace override");
        assert_eq!(
            workspace_read.document.storage_source,
            KnowledgeStorageSource::Project
        );
    }

    #[test]
    fn read_directory_config_with_app_root_marks_app_directory_read_only() {
        let workspace = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();
        let app = TempDir::new().unwrap();
        let app_root = app.path().join("knowledge");
        std::fs::create_dir_all(app_root.join("reference").join("unity")).unwrap();

        let record = read_directory_config_with_app_root(
            &working_dir,
            Some(&app_root),
            KnowledgeType::Reference,
            "unity",
        )
        .expect("read app directory");

        assert_eq!(record.path, "unity");
        assert!(record.read_only);
    }

    #[test]
    fn load_documents_with_app_root_excluding_prefixes_skips_matching_subtree() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(&working_dir, sample_doc()).expect("save design doc");

        let unity_doc = sample_unity_bundle_doc();
        save_document(&working_dir, unity_doc).expect("save unity managed doc");

        let documents = load_documents_with_app_root_excluding_prefixes(
            &working_dir,
            None,
            None,
            None,
            &[(KnowledgeType::Reference, "unity-official-docs".to_string())],
        )
        .expect("load filtered docs");

        assert!(documents.iter().any(|doc| {
            doc.doc_type == KnowledgeType::Design && doc.path == "gameplay/core-loop.md"
        }));
        assert!(!documents.iter().any(|doc| {
            doc.doc_type == KnowledgeType::Reference
                && doc.path == "unity-official-docs/manual/ExecutionOrder.md"
        }));
    }

    #[test]
    fn list_documents_reports_rendered_byte_size() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let doc = sample_doc();

        save_document(&working_dir, doc).expect("save design doc");
        let saved =
            load_document_by_path(&working_dir, KnowledgeType::Design, "gameplay/core-loop.md")
                .expect("load saved doc");
        let expected_size = rendered_document_size_bytes(&saved).expect("rendered size");

        let listed = list_documents(&working_dir, Some(KnowledgeType::Design), None)
            .expect("list design docs");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].byte_size, Some(expected_size));
    }

    #[test]
    fn load_documents_with_app_root_excluding_prefixes_skips_bundle_scan_for_fully_excluded_root() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        save_document(&working_dir, sample_doc()).expect("save design doc");

        let managed_store_path = crate::unity_docs::managed_store_path(&working_dir);
        std::fs::create_dir_all(
            managed_store_path
                .parent()
                .expect("managed store parent directory"),
        )
        .expect("create managed store parent");

        let conn = rusqlite::Connection::open(&managed_store_path).expect("open managed store");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS documents (
                path TEXT PRIMARY KEY,
                doc_id TEXT NOT NULL,
                title TEXT NOT NULL,
                payload_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS directories (
                path TEXT PRIMARY KEY
            )",
        )
        .expect("create managed store schema");
        conn.execute(
            "INSERT INTO documents (path, doc_id, title, payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "unity-official-docs/manual/ExecutionOrder.md",
                "kd_broken_unity_doc",
                "Broken Unity Doc",
                "{not-json}"
            ],
        )
        .expect("seed broken managed row");
        conn.execute(
            "INSERT INTO directories (path) VALUES (?1)",
            rusqlite::params![crate::unity_docs::UNITY_REFERENCE_MANAGED_DIR],
        )
        .expect("seed managed directory row");
        drop(conn);
        std::fs::create_dir_all(knowledge_root(&working_dir).join("reference"))
            .expect("create reference root");
        std::fs::write(
            knowledge_root(&working_dir)
                .join("reference")
                .join("unity_reference_docs_manifest.json"),
            r#"{
  "projectVersion": "2022.3.47f1",
  "docsVersion": "2022.3",
  "locale": "zh-CN",
  "importedAt": 1,
  "importedDocCount": 1,
  "sourceUrl": "https://docs.unity3d.com/cn/2022.3/Manual/OfflineDocumentation.html"
}"#,
        )
        .expect("write managed manifest");

        let documents = load_documents_with_app_root_excluding_prefixes(
            &working_dir,
            None,
            None,
            None,
            &[(KnowledgeType::Reference, "unity-official-docs".to_string())],
        )
        .expect("load docs without touching excluded bundle");

        assert!(documents.iter().any(|doc| {
            doc.doc_type == KnowledgeType::Design && doc.path == "gameplay/core-loop.md"
        }));
        assert!(!documents.iter().any(|doc| {
            doc.doc_type == KnowledgeType::Reference && doc.path.starts_with("unity-official-docs")
        }));
    }
}
