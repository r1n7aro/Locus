use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceSelection {
    #[serde(default)]
    pub branch_ref: Option<String>,
    #[serde(default)]
    pub commits: Vec<String>,
    #[serde(default)]
    pub range: Option<String>,
    /// One-based mainline parent for a merge commit, keyed by the requested ref or resolved OID.
    #[serde(default)]
    pub mainline: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PrepareRequest {
    pub sources: Vec<SourceSelection>,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Exact repository-relative paths. None preserves the full snapshot API.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub mode: PrepareMode,
    #[serde(default)]
    pub eager_catalog: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrepareMode {
    #[default]
    Structural,
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    pub blob: String,
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetSnapshot {
    #[serde(default)]
    pub materialization_epoch: u64,
    pub head: String,
    pub branch: Option<String>,
    pub index_records: String,
    pub index_flags: String,
    pub files: BTreeMap<String, Option<FileState>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitDelta {
    pub commit: String,
    pub parent: String,
    pub source_branch: Option<String>,
    pub path: String,
    pub base: Option<FileState>,
    pub source: Option<FileState>,
    pub kind: String,
    /// The catalog is immutable. IDs are namespaced by commit and file.
    pub changes: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileChoice {
    pub path: String,
    pub version: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub destination: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Selection {
    #[serde(default)]
    pub decisions: BTreeMap<String, Value>,
    #[serde(default)]
    pub files: BTreeMap<String, FileChoice>,
    #[serde(default)]
    pub objects: BTreeMap<String, ObjectChoice>,
    #[serde(default)]
    pub fields: BTreeMap<String, FieldChoice>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectChoice {
    pub path: String,
    pub object_id: String,
    pub commit: String,
    pub operation: String,
    pub resolution: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldChoice {
    pub path: String,
    pub object_id: String,
    pub property_path: String,
    pub commit: Option<String>,
    pub resolution: Value,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeJob {
    pub id: String,
    pub version: u32,
    pub root: String,
    pub project_root: String,
    pub project_id: Option<String>,
    pub snapshot: TargetSnapshot,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub mode: PrepareMode,
    /// Dependency-only inputs are never writable selections. Clean files refer
    /// to pinned Git blobs; dirty inputs retain their actual worktree bytes.
    #[serde(default)]
    pub dependency_files: BTreeMap<String, Option<FileState>>,
    #[serde(default)]
    pub dependency_policy: Option<String>,
    #[serde(default = "legacy_catalog_ready")]
    pub catalog_ready: bool,
    #[serde(default)]
    pub prepare_metrics: Value,
    pub deltas: Vec<CommitDelta>,
    #[serde(default)]
    pub schemas: BTreeMap<String, super::schema::SchemaMap>,
    pub selection: Selection,
    pub revision: u64,
    pub state: String,
    #[serde(default)]
    pub applied_hash: Option<String>,
    #[serde(default)]
    pub applied_files: BTreeMap<String, Option<FileState>>,
    #[serde(default)]
    pub applied_before: BTreeMap<String, Option<FileState>>,
    #[serde(default)]
    pub staged_index_records: Option<String>,
    #[serde(default)]
    pub staged_index_flags: Option<String>,
    #[serde(default)]
    pub unity_validated_hash: Option<String>,
    #[serde(default)]
    pub commit_oid: Option<String>,
    #[serde(default)]
    pub pending_index_before: Option<String>,
    #[serde(default)]
    pub pending_index_after: Option<FileState>,
    #[serde(default)]
    pub commit_validations: BTreeMap<String, CommitValidation>,
}

fn legacy_catalog_ready() -> bool { true }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitValidation {
    pub key: String,
    pub tree: String,
    pub plan_hash: String,
    pub parent: String,
    pub paths: Vec<String>,
    pub include_local_changes: bool,
    pub validated: bool,
    pub details: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preview {
    pub job_id: String,
    pub revision: u64,
    pub plan_hash: String,
    pub files: BTreeMap<String, Option<FileState>>,
    pub issues: Vec<Value>,
    pub excluded_changes: Vec<String>,
    pub deferred_changes: Vec<String>,
    pub ready_to_apply: bool,
    pub unity_validated: bool,
}
