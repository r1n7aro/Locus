pub mod git;
pub mod git_merge;
pub mod undo;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_tree_id: Option<String>,
    pub label: String,
    /// Monotonic unix timestamp in milliseconds used for ordering.
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsRevisionRef {
    pub provider: String,
    pub revision_id: String,
    pub revision_kind: String,
    pub display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsChangedPath {
    pub path: String,
    pub change_kind: String, // "A" | "M" | "D" | "R"
    pub old_path: Option<String>,
}

pub trait VcsProvider: Send + Sync {
    fn checkpoint(
        &self,
        working_dir: &str,
        label: &str,
    ) -> impl std::future::Future<Output = Result<Option<Checkpoint>, String>> + Send;

    fn rollback(
        &self,
        working_dir: &str,
        checkpoint_id: &str,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;

    fn discard(
        &self,
        working_dir: &str,
        checkpoint_id: &str,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;

    fn is_available(&self, working_dir: &str) -> impl std::future::Future<Output = bool> + Send;

    fn name(&self) -> &'static str;

    /// Get the current bindable revision (e.g. HEAD for git).
    /// Returns None if no revision can be bound (empty repo, no VCS).
    fn current_bindable_revision(
        &self,
        working_dir: &str,
    ) -> impl std::future::Future<Output = Option<VcsRevisionRef>> + Send;

    /// Compare two revisions and return changed file paths.
    fn compare_paths(
        &self,
        working_dir: &str,
        from_revision: &str,
        to_revision: &str,
    ) -> impl std::future::Future<Output = Result<Vec<VcsChangedPath>, String>> + Send;
}

pub use git::GitProvider;
pub use undo::UndoManager;
