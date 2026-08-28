use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use serde::Serialize;

use super::ProjectId;

/// Durable checkout materialization used by project-level read projections.
/// A runtime generation is present only while the checkout runtime is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCheckoutSource {
    pub checkout_id: String,
    pub root: PathBuf,
    pub workspace_generation: Option<u64>,
}

fn should_replace_knowledge_materialization(
    candidate_updated_at: i64,
    candidate_checkout_id: &str,
    current_updated_at: i64,
    current_checkout_id: &str,
) -> bool {
    candidate_updated_at > current_updated_at
        || (candidate_updated_at == current_updated_at
            && candidate_checkout_id < current_checkout_id)
}

/// Project-owned catalog that projects the newest materialization of each
/// knowledge document across sibling worktrees. Physical files, watchers and
/// indexes remain owned by their checkout runtime.
pub struct ProjectKnowledgeCatalog {
    project_id: ProjectId,
}

impl ProjectKnowledgeCatalog {
    pub(crate) fn new(project_id: ProjectId) -> Self {
        Self { project_id }
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub fn list_latest(
        &self,
        mut checkouts: Vec<ProjectCheckoutSource>,
        app_knowledge_dir: Option<&PathBuf>,
        doc_type: Option<crate::knowledge_store::KnowledgeType>,
        path_prefix: Option<&str>,
    ) -> Result<Vec<ProjectKnowledgeDocument>, String> {
        checkouts.sort_by(|left, right| left.checkout_id.cmp(&right.checkout_id));
        let mut selected = HashMap::<String, ProjectKnowledgeDocument>::new();

        for (checkout_index, checkout) in checkouts.into_iter().enumerate() {
            let root = checkout.root.to_string_lossy().into_owned();
            let items = crate::knowledge_store::list_documents_with_app_root(
                &root,
                if checkout_index == 0 {
                    app_knowledge_dir
                } else {
                    None
                },
                doc_type,
                path_prefix,
            )?;
            for item in items {
                let key = if item.id.trim().is_empty() {
                    format!("{}:{}", item.doc_type.as_str(), item.path)
                } else {
                    item.id.clone()
                };
                let checkout_id = checkout.checkout_id.clone();
                let candidate = ProjectKnowledgeDocument {
                    item,
                    source_checkout_id: checkout_id.clone(),
                    source_workspace_generation: checkout.workspace_generation,
                    source_root: root.clone(),
                    available_checkout_ids: vec![checkout_id],
                };
                match selected.get_mut(&key) {
                    Some(current) => {
                        let mut available = current
                            .available_checkout_ids
                            .iter()
                            .cloned()
                            .collect::<BTreeSet<_>>();
                        available.extend(candidate.available_checkout_ids.iter().cloned());
                        let replace = should_replace_knowledge_materialization(
                            candidate.item.updated_at,
                            &candidate.source_checkout_id,
                            current.item.updated_at,
                            &current.source_checkout_id,
                        );
                        if replace {
                            *current = candidate;
                        }
                        current.available_checkout_ids = available.into_iter().collect();
                    }
                    None => {
                        selected.insert(key, candidate);
                    }
                }
            }
        }

        let mut documents = selected.into_values().collect::<Vec<_>>();
        documents.sort_by(|left, right| {
            left.item
                .doc_type
                .as_str()
                .cmp(right.item.doc_type.as_str())
                .then(left.item.path.cmp(&right.item.path))
                .then(left.item.title.cmp(&right.item.title))
        });
        Ok(documents)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectKnowledgeDocument {
    #[serde(flatten)]
    pub item: crate::knowledge_store::KnowledgeListItem,
    pub source_checkout_id: String,
    pub source_workspace_generation: Option<u64>,
    pub source_root: String,
    pub available_checkout_ids: Vec<String>,
}

/// Project-owned collaboration root. Repository history and refs are shared;
/// each checkout continues to own HEAD, index, working tree and merge state.
pub struct ProjectCollaborationHub {
    project_id: ProjectId,
}

impl ProjectCollaborationHub {
    pub(crate) fn new(project_id: ProjectId) -> Self {
        Self { project_id }
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub fn snapshot(
        &self,
        mut checkouts: Vec<ProjectCheckoutSource>,
    ) -> ProjectCollaborationSnapshot {
        checkouts.sort_by(|left, right| left.checkout_id.cmp(&right.checkout_id));
        let checkouts = checkouts
            .into_iter()
            .map(|checkout| {
                let root = checkout.root.to_string_lossy().into_owned();
                let head = crate::commands::collect_head_state(&root);
                ProjectCollaborationCheckout {
                    checkout_id: checkout.checkout_id,
                    workspace_generation: checkout.workspace_generation,
                    root,
                    branch_ref: head.ref_name,
                    head_oid: head.hash,
                }
            })
            .collect();
        ProjectCollaborationSnapshot {
            project_id: self.project_id.to_string(),
            checkouts,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCollaborationSnapshot {
    pub project_id: String,
    pub checkouts: Vec<ProjectCollaborationCheckout>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCollaborationCheckout {
    pub checkout_id: String,
    pub workspace_generation: Option<u64>,
    pub root: String,
    pub branch_ref: Option<String>,
    pub head_oid: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        should_replace_knowledge_materialization, ProjectCheckoutSource, ProjectCollaborationHub,
    };
    use crate::workspace_service::ProjectId;

    #[test]
    fn project_knowledge_selection_prefers_newer_then_stable_checkout() {
        assert!(should_replace_knowledge_materialization(
            20,
            "checkout-b",
            10,
            "checkout-a",
        ));
        assert!(!should_replace_knowledge_materialization(
            10,
            "checkout-a",
            20,
            "checkout-b",
        ));
        assert!(should_replace_knowledge_materialization(
            20,
            "checkout-a",
            20,
            "checkout-b",
        ));
        assert!(!should_replace_knowledge_materialization(
            20,
            "checkout-b",
            20,
            "checkout-a",
        ));
    }

    #[test]
    fn collaboration_snapshot_keeps_dormant_checkout_visible() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_id = ProjectId::new("project-durable").expect("project id");
        let hub = ProjectCollaborationHub::new(project_id);

        let snapshot = hub.snapshot(vec![ProjectCheckoutSource {
            checkout_id: "checkout-dormant".to_string(),
            root: temp.path().to_path_buf(),
            workspace_generation: None,
        }]);

        assert_eq!(snapshot.checkouts.len(), 1);
        assert_eq!(snapshot.checkouts[0].checkout_id, "checkout-dormant");
        assert_eq!(snapshot.checkouts[0].workspace_generation, None);
    }
}
