use serde::{Deserialize, Serialize};

fn is_false(v: &bool) -> bool {
    !v
}

// ── Request / Response envelopes ──

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffRequest {
    pub source: DiffSource,
    pub file_path: String,
    pub old_path: Option<String>,
    pub commit_hash: Option<String>,
    pub session_id: Option<String>,
    pub assistant_message_id: Option<String>,
    pub detail: DiffDetail,
    #[serde(default)]
    pub full_context: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffSource {
    GitCommit,
    GitStaged,
    GitUnstaged,
    ChatCheckpoint,
    /// base (stage 1) → ours (stage 2): shows what "our" side changed from common ancestor
    GitConflictBaseToLeft,
    /// base (stage 1) → theirs (stage 3): shows what "their" side changed from common ancestor
    GitConflictBaseToRight,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffDetail {
    Preview,
    Full,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTargetRequest {
    pub diff_key: String,
    pub target_id: String,
    #[serde(default)]
    pub include_unchanged: bool,
}

// ── Diff payload ──

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DiffContentState {
    Normal,
    LfsResolved,
    LfsNotFetched { oid: String, size: u64 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffPayload {
    pub key: String,
    pub file_path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub language: Option<String>,
    pub is_binary: bool,
    pub is_large: bool,
    pub content_state: DiffContentState,
    pub stats: DiffStats,
    pub preview_summary: Vec<String>,
    pub text: Option<TextDiffResult>,
    pub semantic: Option<SemanticDiff>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_preview: Option<BinaryPreview>,
}

// ── Binary preview types ──

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BinaryPreviewKind {
    Image,
    Psd,
    Model,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryAssetRef {
    pub url: String,
    pub mime_type: Option<String>,
    pub byte_size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryPreview {
    pub kind: BinaryPreviewKind,
    pub before: Option<BinaryAssetRef>,
    pub after: Option<BinaryAssetRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffStats {
    pub additions: usize,
    pub deletions: usize,
    pub changed_hunks: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDiffResult {
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub header: String,
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    pub old_line_no: Option<usize>,
    pub new_line_no: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Add,
    Delete,
}

// ── Semantic diff types ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticDiff {
    pub engine: String,
    pub asset_kind: UnityAssetKind,
    pub layout: SemanticLayout,
    pub summary: SemanticSummary,
    pub default_target_id: Option<String>,
    /// Script class name for the main asset (e.g. "PlayerInputConstraint" for ScriptableObjects)
    pub script_class_name: Option<String>,
    pub tree: Option<Vec<SemanticTreeNode>>,
    pub targets: Option<Vec<SemanticTargetSummary>>,
    pub inspector: Option<SemanticTargetInspector>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnityAssetKind {
    Scene,
    Prefab,
    Material,
    ScriptableObject,
    AnimationClip,
    AnimatorController,
    GenericYaml,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SemanticLayout {
    SceneHierarchyInspector,
    AssetInspector,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSummary {
    pub changed_targets: usize,
    pub changed_objects: usize,
    pub changed_components: usize,
    pub changed_fields: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticBadgeCounts {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub components_changed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTreeNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub object_kind: String,
    pub change_kind: String,
    pub path: String,
    pub child_ids: Vec<String>,
    pub badge_counts: SemanticBadgeCounts,
    pub has_inspector: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTargetSummary {
    pub id: String,
    pub label: String,
    pub subtitle: Option<String>,
    pub path: String,
    pub change_kind: String,
    pub has_inspector: bool,
    pub target_kind: Option<String>,
    pub script_class: Option<String>,
    pub is_main_object: Option<bool>,
    pub source_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTargetInspector {
    pub target_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub path: String,
    pub panels: Vec<InspectorPanel>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InspectorPanelKind {
    GameObjectHeader,
    Component,
    AssetRoot,
    SubObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorReference {
    pub guid: Option<String>,
    pub path: Option<String>,
    pub file_id: Option<i64>,
    /// Diagnostic hint when GUID resolution failed (e.g. "not_in_asset_db")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_hint: Option<String>,
    /// `true` when the path was resolved using the current workspace AssetDb
    /// for a snapshot side — the mapping may not match the historical state.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorComponentInference {
    pub reason_code: String,
    pub evidence: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_class_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorPanel {
    pub panel_kind: InspectorPanelKind,
    pub title: String,
    pub script_class: Option<String>,
    pub change_kind: String,
    pub added: bool,
    pub removed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_class_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_resolve_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_inference: Option<InspectorComponentInference>,
    pub fields: Vec<InspectorField>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectorField {
    pub id: String,
    pub label: String,
    pub property_path: String,
    pub value_type: String,
    pub change_kind: String,
    pub before: Option<String>,
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<InspectorField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<InspectorReference>,
    /// C# declared type from script source (e.g. "int", "float", "Color")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
}
