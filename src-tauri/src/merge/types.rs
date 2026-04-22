use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::diff::types::{
    InspectorComponentInference, InspectorPanelKind, InspectorReference, SemanticLayout,
    SemanticTreeNode, UnityAssetKind,
};
use crate::unity_yaml::YamlDoc;

// ── Merge state enums ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeState {
    /// Automatically resolved: only one side changed, or both sides changed identically.
    Auto,
    /// Both sides changed differently from base — needs user decision.
    Conflict,
    /// No change from base on either side.
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeSide {
    Base,
    Ours,
    Theirs,
}

// ── Doc-level merge status ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DocMergeStatus {
    Unchanged,
    AutoResolved,
    HasConflicts,
    AddedOurs,
    AddedTheirs,
    RemovedOurs,
    RemovedTheirs,
}

// ── Merge field (three-way) ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeField {
    pub id: String,
    pub property_path: String,
    pub label: String,
    pub value_type: String,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
    pub result: Option<String>,
    pub merge_state: MergeState,
    pub auto_choice: Option<MergeSide>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_choice: Option<MergeSide>,
    pub children: Vec<MergeField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_base: Option<InspectorReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_ours: Option<InspectorReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_theirs: Option<InspectorReference>,
}

// ── Merge panel ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePanel {
    pub panel_kind: InspectorPanelKind,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_inference: Option<InspectorComponentInference>,
    pub merge_status: DocMergeStatus,
    pub fields: Vec<MergeField>,
}

// ── Merge target (summary + full inspector) ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeTargetSummary {
    pub id: String,
    pub label: String,
    pub path: String,
    pub merge_status: DocMergeStatus,
    pub conflict_count: usize,
    pub auto_resolved_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeTargetInspector {
    pub target_id: String,
    pub title: String,
    pub path: String,
    pub panels: Vec<MergePanel>,
}

// ── Merge summary ──

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeSummary {
    pub total_targets: usize,
    pub conflicting_targets: usize,
    pub auto_resolved_targets: usize,
    pub total_conflicts: usize,
    pub total_auto_resolved: usize,
}

#[derive(Debug, Clone)]
pub(crate) enum MergeTargetLocator {
    SceneTarget { file_id: i64 },
    AssetTarget { match_key: String },
}

// ── Merge session (cached) ──

#[derive(Debug, Clone)]
pub(crate) struct MergeSemanticSession {
    pub(crate) layout: SemanticLayout,
    pub(crate) asset_kind: UnityAssetKind,
    pub(crate) summary: MergeSummary,
    pub(crate) tree: Vec<SemanticTreeNode>,
    pub(crate) targets: Vec<MergeTargetSummary>,
    /// Lazily materialized inspector cache keyed by target id.
    pub(crate) inspectors: HashMap<String, MergeTargetInspector>,
    pub(crate) target_locators: HashMap<String, MergeTargetLocator>,
    /// All unresolved conflict leaf ids for this session.
    pub(crate) conflict_field_ids: HashSet<String>,
    // Parsed docs retained for lazy inspector construction and patch generation.
    pub(crate) base_docs: Vec<YamlDoc>,
    pub(crate) ours_docs: Vec<YamlDoc>,
    pub(crate) theirs_docs: Vec<YamlDoc>,
    pub(crate) base_lines: Vec<String>,
    pub(crate) ours_lines: Vec<String>,
    pub(crate) theirs_lines: Vec<String>,
    /// Hash of the workspace file content at session build time, used to detect
    /// external modifications before applying semantic resolutions.
    pub(crate) workspace_hash: u64,
}

// ── User resolution ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldResolution {
    pub side: MergeSide,
}

// ── IPC request/response types ──

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeSessionRequest {
    pub file_path: String,
    pub base_oid: String,
    pub left_oid: String,
    pub right_oid: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeSessionPayload {
    pub key: String,
    pub file_path: String,
    pub semantic_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_kind: Option<UnityAssetKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<SemanticLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<MergeSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<Vec<SemanticTreeNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<MergeTargetSummary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspector: Option<MergeTargetInspector>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeTargetRequest {
    pub merge_key: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeApplyRequest {
    pub merge_key: String,
    pub file_path: String,
    pub resolutions: HashMap<String, FieldResolution>,
}
