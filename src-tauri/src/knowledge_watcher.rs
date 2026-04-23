use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::AppHandle;

use crate::commands::{self, KnowledgeChangedTarget};
use crate::knowledge_index::KnowledgeIndexState;
use crate::knowledge_store::KnowledgeType;

const WATCHER_BATCH_WINDOW_MS: u64 = 180;
const WATCHER_IDLE_POLL_MS: u64 = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KnowledgeRootKind {
    Workspace,
    App,
}

#[derive(Clone, Debug)]
struct WatchedKnowledgeRoot {
    path: PathBuf,
    kind: KnowledgeRootKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KnowledgeFsChangeKind {
    Content,
    Structure,
    Config,
}

impl KnowledgeFsChangeKind {
    fn event_label(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Structure => "structure",
            Self::Config => "config",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Content => 0,
            Self::Structure => 1,
            Self::Config => 2,
        }
    }

    fn merge(self, other: Self) -> Self {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }
}

#[derive(Clone, Debug)]
enum ResolvedKnowledgeChange {
    Document {
        doc_type: KnowledgeType,
        path: String,
        parent_path: Option<String>,
        change_kind: KnowledgeFsChangeKind,
    },
    Directory {
        doc_type: KnowledgeType,
        path: String,
        parent_path: Option<String>,
        change_kind: KnowledgeFsChangeKind,
        subtree: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ChangeKey {
    Document(KnowledgeType, String),
    Directory(KnowledgeType, String),
}

impl ResolvedKnowledgeChange {
    fn key(&self) -> ChangeKey {
        match self {
            Self::Document { doc_type, path, .. } => ChangeKey::Document(*doc_type, path.clone()),
            Self::Directory { doc_type, path, .. } => ChangeKey::Directory(*doc_type, path.clone()),
        }
    }

    fn merge(self, next: Self) -> Self {
        match (self, next) {
            (
                Self::Document {
                    doc_type,
                    path,
                    parent_path,
                    change_kind,
                },
                Self::Document {
                    change_kind: next_kind,
                    ..
                },
            ) => Self::Document {
                doc_type,
                path,
                parent_path,
                change_kind: change_kind.merge(next_kind),
            },
            (
                Self::Directory {
                    doc_type,
                    path,
                    parent_path,
                    change_kind,
                    subtree,
                },
                Self::Directory {
                    change_kind: next_kind,
                    subtree: next_subtree,
                    ..
                },
            ) => Self::Directory {
                doc_type,
                path,
                parent_path,
                change_kind: change_kind.merge(next_kind),
                subtree: subtree || next_subtree,
            },
            (_, other) => other,
        }
    }
}

pub struct KnowledgeFsWatcher {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _os_watcher: RecommendedWatcher,
}

impl KnowledgeFsWatcher {
    pub fn start(
        app_handle: AppHandle,
        working_dir: String,
        app_knowledge_dir: Option<PathBuf>,
        knowledge_index_state: Arc<KnowledgeIndexState>,
    ) -> Result<Self, String> {
        let roots = watched_roots(&working_dir, app_knowledge_dir);
        if roots.is_empty() {
            return Err("No knowledge roots available to watch".to_string());
        }

        let (tx, rx) = mpsc::channel();
        let mut os_watcher = RecommendedWatcher::new(tx, Config::default())
            .map_err(|e| format!("Failed to create knowledge watcher: {}", e))?;
        for root in &roots {
            os_watcher
                .watch(&root.path, RecursiveMode::Recursive)
                .map_err(|e| {
                    format!(
                        "Failed to watch knowledge root '{}': {}",
                        root.path.display(),
                        e
                    )
                })?;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("knowledge-fs-watcher".to_string())
            .spawn(move || {
                watcher_loop(
                    rx,
                    worker_stop,
                    app_handle,
                    working_dir,
                    knowledge_index_state,
                    roots,
                );
            })
            .map_err(|e| format!("Failed to spawn knowledge watcher thread: {}", e))?;

        Ok(Self {
            stop,
            worker: Some(worker),
            _os_watcher: os_watcher,
        })
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn watched_roots(
    working_dir: &str,
    app_knowledge_dir: Option<PathBuf>,
) -> Vec<WatchedKnowledgeRoot> {
    let mut roots = Vec::new();
    let workspace_root = crate::knowledge_store::knowledge_root(working_dir);
    if workspace_root.is_dir() {
        roots.push(WatchedKnowledgeRoot {
            path: workspace_root,
            kind: KnowledgeRootKind::Workspace,
        });
    }
    if let Some(app_root) = app_knowledge_dir {
        if app_root.is_dir() {
            let is_duplicate = roots
                .iter()
                .any(|existing| same_path(&existing.path, &app_root));
            if !is_duplicate {
                roots.push(WatchedKnowledgeRoot {
                    path: app_root,
                    kind: KnowledgeRootKind::App,
                });
            }
        }
    }
    roots
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = left.to_string_lossy().replace('\\', "/").to_lowercase();
    let right = right.to_string_lossy().replace('\\', "/").to_lowercase();
    left == right
}

fn watcher_loop(
    rx: mpsc::Receiver<notify::Result<Event>>,
    stop: Arc<AtomicBool>,
    app_handle: AppHandle,
    working_dir: String,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    roots: Vec<WatchedKnowledgeRoot>,
) {
    while !stop.load(Ordering::Relaxed) {
        let first = match rx.recv_timeout(Duration::from_millis(WATCHER_IDLE_POLL_MS)) {
            Ok(event) => Some(event),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        let Some(first) = first else {
            continue;
        };

        let mut batch = vec![first];
        let deadline = Instant::now() + Duration::from_millis(WATCHER_BATCH_WINDOW_MS);
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining) {
                Ok(event) => batch.push(event),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let changes = collect_batch_changes(&batch, &roots);
        if changes.is_empty() {
            continue;
        }

        if let Err(error) = tauri::async_runtime::block_on(process_batch_changes(
            &app_handle,
            &working_dir,
            knowledge_index_state.clone(),
            changes,
        )) {
            eprintln!(
                "[KnowledgeWatcher] failed to process change batch: {}",
                error
            );
        }
    }
}

fn collect_batch_changes(
    batch: &[notify::Result<Event>],
    roots: &[WatchedKnowledgeRoot],
) -> Vec<ResolvedKnowledgeChange> {
    let mut changes = HashMap::<ChangeKey, ResolvedKnowledgeChange>::new();

    for entry in batch {
        let event = match entry {
            Ok(event) => event,
            Err(error) => {
                eprintln!("[KnowledgeWatcher] notify error: {}", error);
                continue;
            }
        };
        for resolved in resolve_event_changes(event, roots) {
            let key = resolved.key();
            changes
                .entry(key)
                .and_modify(|existing| {
                    *existing = existing.clone().merge(resolved.clone());
                })
                .or_insert(resolved);
        }
    }

    changes.into_values().collect()
}

fn resolve_event_changes(
    event: &Event,
    roots: &[WatchedKnowledgeRoot],
) -> Vec<ResolvedKnowledgeChange> {
    let mut changes = Vec::new();
    for path in &event.paths {
        let Some(change_kind) = classify_change_kind(&event.kind, path) else {
            continue;
        };
        for root in roots {
            let Some(resolved) = resolve_path_change(path, root, change_kind) else {
                continue;
            };
            changes.push(resolved);
            break;
        }
    }
    changes
}

fn classify_change_kind(kind: &EventKind, path: &Path) -> Option<KnowledgeFsChangeKind> {
    if is_directory_config_path(path) {
        return Some(KnowledgeFsChangeKind::Config);
    }

    match kind {
        EventKind::Access(_) => None,
        EventKind::Any => Some(KnowledgeFsChangeKind::Structure),
        EventKind::Create(CreateKind::Any)
        | EventKind::Create(CreateKind::File)
        | EventKind::Create(CreateKind::Folder)
        | EventKind::Create(CreateKind::Other) => Some(KnowledgeFsChangeKind::Structure),
        EventKind::Remove(RemoveKind::Any)
        | EventKind::Remove(RemoveKind::File)
        | EventKind::Remove(RemoveKind::Folder)
        | EventKind::Remove(RemoveKind::Other) => Some(KnowledgeFsChangeKind::Structure),
        EventKind::Modify(ModifyKind::Name(RenameMode::Any))
        | EventKind::Modify(ModifyKind::Name(RenameMode::Both))
        | EventKind::Modify(ModifyKind::Name(RenameMode::From))
        | EventKind::Modify(ModifyKind::Name(RenameMode::To))
        | EventKind::Modify(ModifyKind::Name(RenameMode::Other))
        | EventKind::Modify(ModifyKind::Any) => Some(KnowledgeFsChangeKind::Structure),
        EventKind::Modify(ModifyKind::Data(_)) | EventKind::Modify(ModifyKind::Metadata(_)) => {
            Some(KnowledgeFsChangeKind::Content)
        }
        _ => {
            if is_markdown_path(path) {
                Some(KnowledgeFsChangeKind::Content)
            } else {
                None
            }
        }
    }
}

fn resolve_path_change(
    path: &Path,
    root: &WatchedKnowledgeRoot,
    change_kind: KnowledgeFsChangeKind,
) -> Option<ResolvedKnowledgeChange> {
    let relative = path.strip_prefix(&root.path).ok()?;
    let mut components = relative.components();
    let type_component = match components.next()? {
        Component::Normal(value) => value.to_string_lossy().to_string(),
        _ => return None,
    };
    let doc_type = parse_knowledge_type(&type_component)?;
    let within_type = path_components_to_slash(components);

    if within_type.is_empty() {
        return None;
    }

    if is_markdown_path(path) {
        let parent_path = parent_directory(&within_type);
        return Some(ResolvedKnowledgeChange::Document {
            doc_type,
            path: within_type,
            parent_path,
            change_kind,
        });
    }

    if is_directory_config_path(path) {
        let directory_path = directory_path_from_sidecar(&within_type)?;
        let parent_path = parent_directory(&directory_path);
        return Some(ResolvedKnowledgeChange::Directory {
            doc_type,
            path: directory_path,
            parent_path,
            change_kind: KnowledgeFsChangeKind::Config,
            subtree: true,
        });
    }

    if change_kind == KnowledgeFsChangeKind::Structure
        && (path.is_dir() || path.extension().is_none())
        && root.kind == KnowledgeRootKind::Workspace
    {
        let parent_path = parent_directory(&within_type);
        return Some(ResolvedKnowledgeChange::Directory {
            doc_type,
            path: within_type,
            parent_path,
            change_kind,
            subtree: true,
        });
    }

    None
}

fn path_components_to_slash<'a>(components: impl Iterator<Item = Component<'a>>) -> String {
    let mut out = Vec::new();
    for component in components {
        let Component::Normal(value) = component else {
            continue;
        };
        let trimmed = value.to_string_lossy().trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        out.push(trimmed);
    }
    out.join("/")
}

fn parse_knowledge_type(value: &str) -> Option<KnowledgeType> {
    match value {
        "design" => Some(KnowledgeType::Design),
        "memory" => Some(KnowledgeType::Memory),
        "skill" => Some(KnowledgeType::Skill),
        "reference" => Some(KnowledgeType::Reference),
        _ => None,
    }
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn is_directory_config_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.ends_with(".locus-meta") || value.ends_with(".meta"))
        .unwrap_or(false)
}

fn directory_path_from_sidecar(relative_path: &str) -> Option<String> {
    let normalized = relative_path.replace('\\', "/");
    let file_name = Path::new(&normalized).file_name()?.to_string_lossy();
    let dir_name = if let Some(stripped) = file_name.strip_suffix(".locus-meta") {
        stripped
    } else if let Some(stripped) = file_name.strip_suffix(".meta") {
        stripped
    } else {
        return None;
    };
    let parent = Path::new(&normalized)
        .parent()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let parent = parent.trim_matches('/');
    Some(if parent.is_empty() {
        dir_name.to_string()
    } else {
        format!("{}/{}", parent, dir_name)
    })
}

fn parent_directory(path: &str) -> Option<String> {
    let parent = Path::new(path).parent()?;
    let normalized = parent.to_string_lossy().replace('\\', "/");
    let trimmed = normalized.trim_matches('/').trim();
    if trimmed.is_empty() || trimmed == "." {
        None
    } else {
        Some(trimmed.to_string())
    }
}

async fn process_batch_changes(
    app_handle: &AppHandle,
    working_dir: &str,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    changes: Vec<ResolvedKnowledgeChange>,
) -> Result<(), String> {
    for change in changes {
        match &change {
            ResolvedKnowledgeChange::Document { doc_type, path, .. } => {
                commands::sync_visible_document_for_path(
                    app_handle,
                    working_dir,
                    knowledge_index_state.clone(),
                    *doc_type,
                    path,
                )
                .await
                .map_err(|error| error.to_string())?;
            }
            ResolvedKnowledgeChange::Directory { doc_type, path, .. } => {
                if path.trim().is_empty() {
                    commands::reconcile_and_emit_knowledge_changed(
                        app_handle,
                        working_dir,
                        knowledge_index_state.clone(),
                        "knowledge_fs_watcher",
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                    continue;
                }
                commands::sync_visible_documents_for_prefix(
                    app_handle,
                    working_dir,
                    knowledge_index_state.clone(),
                    *doc_type,
                    path,
                )
                .await
                .map_err(|error| error.to_string())?;
            }
        }

        emit_change_event(app_handle, working_dir, &change);
    }
    Ok(())
}

fn emit_change_event(app_handle: &AppHandle, working_dir: &str, change: &ResolvedKnowledgeChange) {
    let target = match change {
        ResolvedKnowledgeChange::Document {
            doc_type,
            path,
            parent_path,
            change_kind,
        } => KnowledgeChangedTarget {
            doc_type: Some(*doc_type),
            path: Some(path.clone()),
            parent_path: parent_path.clone(),
            target_kind: Some("document"),
            change_kind: Some(change_kind.event_label()),
            subtree: false,
        },
        ResolvedKnowledgeChange::Directory {
            doc_type,
            path,
            parent_path,
            change_kind,
            subtree,
        } => KnowledgeChangedTarget {
            doc_type: Some(*doc_type),
            path: Some(path.clone()),
            parent_path: parent_path.clone(),
            target_kind: Some("directory"),
            change_kind: Some(change_kind.event_label()),
            subtree: *subtree,
        },
    };
    commands::emit_knowledge_changed_with_target(
        app_handle,
        working_dir,
        "knowledge_fs_watcher",
        target,
    );
}

#[cfg(test)]
mod tests {
    use super::{directory_path_from_sidecar, parent_directory, parse_knowledge_type};
    use crate::knowledge_store::KnowledgeType;

    #[test]
    fn parses_directory_path_from_locus_meta_sidecar() {
        assert_eq!(
            directory_path_from_sidecar("combat/notes.locus-meta"),
            Some("combat/notes".to_string())
        );
        assert_eq!(
            directory_path_from_sidecar("combat/notes.meta"),
            Some("combat/notes".to_string())
        );
    }

    #[test]
    fn parses_parent_directory() {
        assert_eq!(
            parent_directory("combat/core-loop.md"),
            Some("combat".to_string())
        );
        assert_eq!(parent_directory("core-loop.md"), None);
    }

    #[test]
    fn parses_knowledge_type_from_root_segment() {
        assert_eq!(parse_knowledge_type("design"), Some(KnowledgeType::Design));
        assert_eq!(parse_knowledge_type("memory"), Some(KnowledgeType::Memory));
        assert_eq!(parse_knowledge_type("skill"), Some(KnowledgeType::Skill));
        assert_eq!(
            parse_knowledge_type("reference"),
            Some(KnowledgeType::Reference)
        );
        assert_eq!(parse_knowledge_type("unknown"), None);
    }
}
