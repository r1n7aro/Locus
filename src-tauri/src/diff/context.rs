use crate::asset_db::types::Guid;
use crate::asset_db::AssetDbState;
use serde::Serialize;

use super::types::DiffSource;

/// Controls what data each side of a semantic diff can access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceMode {
    /// Pure snapshot — no current workspace state used.
    Snapshot,
    /// Use current workspace AssetDb for reference resolution.
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideFileSource {
    Workspace,
    GitRef(String),
    GitIndex,
    /// A specific conflict stage in the git index (1=base, 2=ours, 3=theirs).
    GitStage(u8),
}

/// Per-side context that controls reference resolution boundaries.
pub struct SideContext<'a> {
    pub guid_resolver: GuidResolver<'a>,
    pub script_guid_resolver: GuidResolver<'a>,
    pub source_mode: SourceMode,
    pub file_source: SideFileSource,
}

/// How to resolve a GUID → asset path on this side.
pub enum GuidResolver<'a> {
    /// Use current AssetDb.
    Workspace(&'a AssetDbState),
    /// Do not resolve — return None for all lookups.
    None,
}

impl<'a> SideContext<'a> {
    /// Resolve a GUID to a file path, respecting source isolation.
    pub fn resolve_guid_path(&self, guid: &Guid) -> Option<String> {
        match &self.guid_resolver {
            GuidResolver::Workspace(ref_graph_state) => {
                let guard = ref_graph_state.0.lock().ok()?;
                let graph = guard.as_ref()?;
                graph.resolve_path_by_guid(guid).ok().flatten()
            }
            GuidResolver::None => None,
        }
    }

    /// Returns true if this side reads from a snapshot (not the live workspace).
    /// GUID paths resolved on a snapshot side are best-effort: the workspace
    /// AssetDb may not match the historical state, so callers should mark the
    /// result as `stale`.
    pub fn is_snapshot(&self) -> bool {
        self.source_mode == SourceMode::Snapshot
    }

    /// Resolve a script GUID to a file path. This is kept separate from normal
    /// asset reference resolution so semantic diff can load the matching C#
    /// definition without changing snapshot-side display behavior.
    pub fn resolve_script_guid_path(&self, guid: &Guid) -> Option<String> {
        match &self.script_guid_resolver {
            GuidResolver::Workspace(ref_graph_state) => {
                let guard = ref_graph_state.0.lock().ok()?;
                let graph = guard.as_ref()?;
                graph.resolve_path_by_guid(guid).ok().flatten()
            }
            GuidResolver::None => None,
        }
    }
}

/// Dual-side context for semantic diff construction.
/// Each side has its own isolation boundary based on DiffSource.
pub struct DiffBuildContext<'a> {
    pub source: DiffSource,
    pub old: SideContext<'a>,
    pub new: SideContext<'a>,
}

impl<'a> DiffBuildContext<'a> {
    /// Construct a context from the diff source, enforcing isolation rules:
    ///
    /// | DiffSource      | old side              | new side              |
    /// |-----------------|---------------------- |---------------------- |
    /// | GitCommit       | Snapshot (best-effort)| Snapshot (best-effort)|
    /// | GitStaged       | Snapshot (best-effort)| Snapshot (best-effort)|
    /// | GitUnstaged     | Snapshot (best-effort)| Workspace             |
    /// | ChatCheckpoint  | Snapshot (best-effort)| Workspace             |
    pub fn from_sources(
        source: DiffSource,
        ref_graph: &'a AssetDbState,
        old_file_source: SideFileSource,
        new_file_source: SideFileSource,
    ) -> Self {
        match source {
            DiffSource::GitCommit | DiffSource::GitStaged => Self {
                source,
                old: SideContext {
                    guid_resolver: GuidResolver::Workspace(ref_graph),
                    script_guid_resolver: GuidResolver::Workspace(ref_graph),
                    source_mode: SourceMode::Snapshot,
                    file_source: old_file_source,
                },
                new: SideContext {
                    guid_resolver: GuidResolver::Workspace(ref_graph),
                    script_guid_resolver: GuidResolver::Workspace(ref_graph),
                    source_mode: SourceMode::Snapshot,
                    file_source: new_file_source,
                },
            },
            DiffSource::GitUnstaged | DiffSource::ChatCheckpoint => Self {
                source,
                old: SideContext {
                    guid_resolver: GuidResolver::Workspace(ref_graph),
                    script_guid_resolver: GuidResolver::Workspace(ref_graph),
                    source_mode: SourceMode::Snapshot,
                    file_source: old_file_source,
                },
                new: SideContext {
                    guid_resolver: GuidResolver::Workspace(ref_graph),
                    script_guid_resolver: GuidResolver::Workspace(ref_graph),
                    source_mode: SourceMode::Workspace,
                    file_source: new_file_source,
                },
            },
            // Conflict diffs: both sides are index stages (snapshot), not workspace
            DiffSource::GitConflictBaseToLeft | DiffSource::GitConflictBaseToRight => Self {
                source,
                old: SideContext {
                    guid_resolver: GuidResolver::Workspace(ref_graph),
                    script_guid_resolver: GuidResolver::Workspace(ref_graph),
                    source_mode: SourceMode::Snapshot,
                    file_source: old_file_source,
                },
                new: SideContext {
                    guid_resolver: GuidResolver::Workspace(ref_graph),
                    script_guid_resolver: GuidResolver::Workspace(ref_graph),
                    source_mode: SourceMode::Snapshot,
                    file_source: new_file_source,
                },
            },
        }
    }
}
