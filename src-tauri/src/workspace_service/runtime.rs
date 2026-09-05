use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Instant;

use serde::Serialize;
use tokio::sync::Notify;

use crate::resource_policy::{ResourcePolicyStore, WorkspaceActivityPriority};

use super::event::WorkspaceEventRouter;
use super::execution::AgentExecutionContext;
use super::identity::{CheckoutId, ProjectId, ProjectIdResolver, ResolvedWorkspaceIdentity};
use super::scope::{ResolvedWorkspaceScope, WorkspaceRef, WorkspaceResolveError};
use super::service::{ServiceBinding, ServiceKind, WorkspaceServiceFactory, WorkspaceServiceHost};

#[derive(Debug)]
pub struct WorkspaceLeaseTracker {
    count: AtomicUsize,
    running_tasks: AtomicUsize,
    visible_panes: AtomicUsize,
    background_open: AtomicUsize,
    last_used_at: Mutex<Instant>,
}

impl Default for WorkspaceLeaseTracker {
    fn default() -> Self {
        Self {
            count: AtomicUsize::new(0),
            running_tasks: AtomicUsize::new(0),
            visible_panes: AtomicUsize::new(0),
            background_open: AtomicUsize::new(0),
            last_used_at: Mutex::new(Instant::now()),
        }
    }
}

impl WorkspaceLeaseTracker {
    fn acquire(self: &Arc<Self>, kind: WorkspaceLeaseKind) -> WorkspaceLeaseToken {
        self.count.fetch_add(1, Ordering::AcqRel);
        match kind {
            WorkspaceLeaseKind::RunningTask => {
                self.running_tasks.fetch_add(1, Ordering::AcqRel);
            }
            WorkspaceLeaseKind::VisiblePane => {
                self.visible_panes.fetch_add(1, Ordering::AcqRel);
            }
            WorkspaceLeaseKind::BackgroundOpen => {
                self.background_open.fetch_add(1, Ordering::AcqRel);
            }
        }
        if let Ok(mut last_used_at) = self.last_used_at.lock() {
            *last_used_at = Instant::now();
        }
        WorkspaceLeaseToken {
            tracker: Arc::clone(self),
            kind,
        }
    }

    pub fn count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }

    pub fn idle_for(&self) -> std::time::Duration {
        self.last_used_at
            .lock()
            .map(|last_used_at| last_used_at.elapsed())
            .unwrap_or_default()
    }

    fn snapshot(&self, idle_timeout: std::time::Duration) -> WorkspaceActivitySnapshot {
        let recorded_running_tasks = self.running_tasks.load(Ordering::Acquire);
        let visible_pane_leases = self.visible_panes.load(Ordering::Acquire);
        let background_open_leases = self.background_open.load(Ordering::Acquire);
        let total_leases = self.count.load(Ordering::Acquire);
        // Lease acquisition/release updates the aggregate and typed counters
        // separately. Treat any in-flight, not-yet-classified lease as a task
        // so convergence can only become more conservative during that tiny
        // transition window.
        let running_task_leases = recorded_running_tasks
            .max(total_leases.saturating_sub(visible_pane_leases + background_open_leases));
        let idle_for = self.idle_for();
        let priority = if running_task_leases > 0 {
            WorkspaceActivityPriority::RunningTask
        } else if visible_pane_leases > 0 {
            WorkspaceActivityPriority::VisiblePane
        } else if background_open_leases > 0 {
            WorkspaceActivityPriority::BackgroundOpen
        } else if idle_for >= idle_timeout {
            WorkspaceActivityPriority::Idle
        } else {
            WorkspaceActivityPriority::BackgroundOpen
        };
        WorkspaceActivitySnapshot {
            priority,
            running_task_leases,
            visible_pane_leases,
            background_open_leases,
            idle_for,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceLeaseKind {
    RunningTask,
    VisiblePane,
    BackgroundOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceActivitySnapshot {
    pub priority: WorkspaceActivityPriority,
    pub running_task_leases: usize,
    pub visible_pane_leases: usize,
    pub background_open_leases: usize,
    pub idle_for: std::time::Duration,
}

struct WorkspaceLeaseToken {
    tracker: Arc<WorkspaceLeaseTracker>,
    kind: WorkspaceLeaseKind,
}

impl Drop for WorkspaceLeaseToken {
    fn drop(&mut self) {
        match self.kind {
            WorkspaceLeaseKind::RunningTask => {
                self.tracker.running_tasks.fetch_sub(1, Ordering::AcqRel);
            }
            WorkspaceLeaseKind::VisiblePane => {
                self.tracker.visible_panes.fetch_sub(1, Ordering::AcqRel);
            }
            WorkspaceLeaseKind::BackgroundOpen => {
                self.tracker.background_open.fetch_sub(1, Ordering::AcqRel);
            }
        }
        self.tracker.count.fetch_sub(1, Ordering::AcqRel);
        if let Ok(mut last_used_at) = self.tracker.last_used_at.lock() {
            *last_used_at = Instant::now();
        }
    }
}

/// A cloneable lease handle. Clones share one logical run lease and release it
/// only after every background task carrying that run context has finished.
#[derive(Clone)]
pub struct WorkspaceLease {
    _token: Arc<WorkspaceLeaseToken>,
}

#[derive(Clone)]
pub struct WorkspaceCoreServices {
    root: PathBuf,
    repository_key: String,
    asset_db: Arc<std::sync::Mutex<Option<crate::asset_db::AssetDb>>>,
    knowledge_index:
        Arc<std::sync::Mutex<Option<Arc<crate::knowledge_index::KnowledgeIndexState>>>>,
    asset_watcher: Arc<std::sync::Mutex<Option<crate::asset_db::watcher::AssetDbWatcher>>>,
    workspace_changes: Arc<crate::workspace_changes::WorkspaceChangeHub>,
    knowledge_watcher: Arc<std::sync::Mutex<Option<crate::knowledge_watcher::KnowledgeFsWatcher>>>,
    background_watchers_lifecycle: Arc<std::sync::Mutex<()>>,
    knowledge_operations: WorkspaceKnowledgeOperationStates,
    asset_last_scan_info: crate::commands::asset::LastScanInfoState,
    asset_scan_phase: crate::commands::asset::ScanPhaseState,
    asset_preview_cache: Arc<crate::commands::asset::WorkspacePreviewCache>,
    dir_entries_page_cache: crate::commands::DirEntriesPageCache,
    ref_graph_scan_tasks: crate::commands::RefGraphScanTaskState,
    asset_reconcile_tasks: crate::commands::asset::AssetDbReconcileTaskState,
}

/// Mutable jobs owned by one checkout's Knowledge data plane. Keeping these
/// handles on the runtime prevents imports and watchers in sibling worktrees
/// from sharing cancellation/status state.
#[derive(Clone, Default)]
pub struct WorkspaceKnowledgeOperationStates {
    pub unity_reference_import: crate::unity_docs::UnityReferenceImportState,
    pub feishu_reference_import: crate::feishu_docs::FeishuReferenceImportState,
    pub local_reference_import: crate::local_docs::LocalReferenceImportState,
    pub local_reference_watcher: crate::local_docs::LocalReferenceWatcherState,
}

impl WorkspaceCoreServices {
    fn new(identity: &ResolvedWorkspaceIdentity) -> Self {
        let repository_key = identity
            .git_common_dir
            .as_ref()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| identity.normalized_root.clone());
        let asset_db = match crate::asset_db::AssetDb::load_existing(&identity.root) {
            crate::asset_db::LoadExistingAssetDb::Ready(asset_db) => Some(asset_db),
            crate::asset_db::LoadExistingAssetDb::Missing
            | crate::asset_db::LoadExistingAssetDb::NeedsRescan(_) => None,
        };
        Self {
            root: identity.root.clone(),
            repository_key,
            asset_db: Arc::new(std::sync::Mutex::new(asset_db)),
            knowledge_index: Arc::new(std::sync::Mutex::new(None)),
            asset_watcher: Arc::new(std::sync::Mutex::new(None)),
            workspace_changes: crate::workspace_changes::hub_for_workspace(&identity.root),
            knowledge_watcher: Arc::new(std::sync::Mutex::new(None)),
            background_watchers_lifecycle: Arc::new(std::sync::Mutex::new(())),
            knowledge_operations: WorkspaceKnowledgeOperationStates::default(),
            asset_last_scan_info: crate::commands::asset::LastScanInfoState::new(),
            asset_scan_phase: crate::commands::asset::ScanPhaseState::new(),
            asset_preview_cache: Arc::new(crate::commands::asset::WorkspacePreviewCache::new()),
            dir_entries_page_cache: crate::commands::DirEntriesPageCache::new(),
            ref_graph_scan_tasks: crate::commands::RefGraphScanTaskState::new(),
            asset_reconcile_tasks: crate::commands::asset::AssetDbReconcileTaskState::new(),
        }
    }

    pub fn workspace_fs_lock_name(&self, checkout_id: &CheckoutId) -> String {
        format!("workspace:{}:fs", checkout_id.as_str())
    }

    pub fn repository_lock_name(&self) -> String {
        format!("repository:{}", self.repository_key)
    }

    pub fn asset_db(&self) -> Arc<std::sync::Mutex<Option<crate::asset_db::AssetDb>>> {
        Arc::clone(&self.asset_db)
    }

    pub fn workspace_changes(&self) -> Arc<crate::workspace_changes::WorkspaceChangeHub> {
        Arc::clone(&self.workspace_changes)
    }

    pub fn asset_watcher_diagnostics(&self) -> (bool, u64, Option<String>) {
        let Ok(watcher) = self.asset_watcher.lock() else {
            return (false, 0, None);
        };
        match watcher.as_ref() {
            Some(watcher) => (true, watcher.queue_len() as u64, watcher.current_file()),
            None => (false, 0, None),
        }
    }

    pub fn asset_last_scan_info(&self) -> &crate::commands::asset::LastScanInfoState {
        &self.asset_last_scan_info
    }

    pub fn asset_scan_phase(&self) -> &crate::commands::asset::ScanPhaseState {
        &self.asset_scan_phase
    }

    pub fn asset_preview_cache(&self) -> &Arc<crate::commands::asset::WorkspacePreviewCache> {
        &self.asset_preview_cache
    }

    pub fn dir_entries_page_cache(&self) -> &crate::commands::DirEntriesPageCache {
        &self.dir_entries_page_cache
    }

    pub fn ref_graph_scan_tasks(&self) -> &crate::commands::RefGraphScanTaskState {
        &self.ref_graph_scan_tasks
    }

    pub fn asset_reconcile_tasks(&self) -> &crate::commands::asset::AssetDbReconcileTaskState {
        &self.asset_reconcile_tasks
    }

    pub fn refresh_asset_db_if_missing(&self, root: &Path) {
        let Ok(mut current) = self.asset_db.lock() else {
            return;
        };
        if current.is_some() {
            return;
        }
        if let crate::asset_db::LoadExistingAssetDb::Ready(asset_db) =
            crate::asset_db::AssetDb::load_existing(root)
        {
            *current = Some(asset_db);
        }
    }

    fn reload_asset_db(&self) {
        let Ok(mut current) = self.asset_db.lock() else {
            return;
        };
        if let crate::asset_db::LoadExistingAssetDb::Ready(asset_db) =
            crate::asset_db::AssetDb::load_existing(&self.root)
        {
            *current = Some(asset_db);
        }
    }

    fn knowledge_index(
        &self,
        event_scope: crate::workspace_service::event::WorkspaceEventScope,
        app_handle: &tauri::AppHandle,
    ) -> Result<Arc<crate::knowledge_index::KnowledgeIndexState>, String> {
        let mut current = self
            .knowledge_index
            .lock()
            .map_err(|error| format!("workspace knowledge index lock poisoned: {error}"))?;
        if let Some(index) = current.as_ref() {
            return Ok(Arc::clone(index));
        }
        let library_dir =
            crate::knowledge_index::library_dir_for_working_dir(&self.root.to_string_lossy());
        let storage_dir = crate::commands::resolve_runtime_storage_dir(app_handle)?;
        let runtime = crate::knowledge_index::KnowledgeRuntime::open(&library_dir, &storage_dir)
            .map_err(|error| format!("Failed to initialize workspace knowledge index: {error}"))?;
        let index = Arc::new(
            crate::knowledge_index::KnowledgeIndexState::new_with_workspace_app_handle(
                runtime.db,
                runtime.tantivy,
                runtime.embedding_mgr,
                app_handle.clone(),
                event_scope,
            ),
        );
        *current = Some(Arc::clone(&index));
        Ok(index)
    }

    pub fn knowledge_operations(&self) -> WorkspaceKnowledgeOperationStates {
        self.knowledge_operations.clone()
    }

    pub fn watchers_running(&self) -> bool {
        self.asset_watcher
            .lock()
            .map(|watcher| watcher.is_some())
            .unwrap_or(false)
            || self
                .knowledge_watcher
                .lock()
                .map(|watcher| watcher.is_some())
                .unwrap_or(false)
            || self
                .knowledge_operations
                .local_reference_watcher
                .live_watcher_count()
                > 0
    }

    /// Keep an unselected runtime live after the legacy facade moves away.
    /// App-level knowledge is intentionally excluded here so only the selected
    /// compatibility watcher observes that shared root.
    pub fn start_background_watchers(
        &self,
        runtime: &WorkspaceRuntime,
        app_handle: &tauri::AppHandle,
        watcher_tuning: Arc<crate::asset_db::watcher::WatcherTuning>,
    ) -> Result<(), String> {
        let _lifecycle = self
            .background_watchers_lifecycle
            .lock()
            .map_err(|error| format!("workspace watcher lifecycle lock poisoned: {error}"))?;
        self.reload_asset_db();
        {
            let mut watcher = self
                .asset_watcher
                .lock()
                .map_err(|error| format!("workspace asset watcher lock poisoned: {error}"))?;
            if watcher.is_none() && self.root.join("Assets").is_dir() {
                *watcher = Some(crate::asset_db::watcher::AssetDbWatcher::start(
                    self.root.clone(),
                    Arc::clone(&self.asset_db),
                    watcher_tuning,
                    Arc::clone(&self.workspace_changes),
                    app_handle.clone(),
                    crate::workspace_service::event::WorkspaceEventScope::for_runtime(runtime),
                )?);
            }
        }
        let mut knowledge_watcher_started = false;
        {
            let mut watcher = self
                .knowledge_watcher
                .lock()
                .map_err(|error| format!("workspace knowledge watcher lock poisoned: {error}"))?;
            if watcher.is_none()
                && crate::knowledge_store::knowledge_root(&self.root.to_string_lossy()).is_dir()
            {
                *watcher = Some(crate::knowledge_watcher::KnowledgeFsWatcher::start(
                    app_handle.clone(),
                    self.root.to_string_lossy().to_string(),
                    crate::workspace_service::event::WorkspaceEventScope::for_runtime(runtime),
                    None,
                    runtime.knowledge_index(app_handle)?,
                )?);
                knowledge_watcher_started = true;
            }
        }
        if knowledge_watcher_started {
            crate::local_docs::restore_live_watchers(
                app_handle.clone(),
                self.root.to_string_lossy().to_string(),
                runtime.knowledge_index(app_handle)?,
                self.knowledge_operations.local_reference_watcher.clone(),
            );
        }
        Ok(())
    }

    pub fn stop_background_watchers(&self) {
        let _lifecycle = self
            .background_watchers_lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Ok(mut watcher) = self.asset_watcher.lock() {
            if let Some(watcher) = watcher.take() {
                watcher.stop_and_join();
            }
        }
        // Live-link workers can reconcile the Tantivy index. Join them before
        // dropping the filesystem watcher and index owner so a replacement
        // runtime cannot race a worker from the retired generation.
        crate::local_docs::clear_live_watchers(&self.knowledge_operations.local_reference_watcher);
        if let Ok(mut watcher) = self.knowledge_watcher.lock() {
            if let Some(watcher) = watcher.take() {
                watcher.stop();
            }
        }
        if let Ok(mut index) = self.knowledge_index.lock() {
            index.take();
        }
    }
}

pub struct WorkspaceRuntime {
    project_id: ProjectId,
    checkout_id: CheckoutId,
    root: PathBuf,
    normalized_root: String,
    generation: AtomicU64,
    core: WorkspaceCoreServices,
    services: Arc<WorkspaceServiceHost>,
    leases: Arc<WorkspaceLeaseTracker>,
    definition_cache: Arc<crate::workspace_definition_registry::DefinitionCacheEntry>,
}

impl WorkspaceRuntime {
    pub(crate) fn new(
        identity: ResolvedWorkspaceIdentity,
        factories: Vec<Arc<dyn WorkspaceServiceFactory>>,
        generation: u64,
    ) -> Arc<Self> {
        let services = Arc::new(WorkspaceServiceHost::new(factories));
        let runtime = Arc::new(Self {
            project_id: identity.project_id.clone(),
            checkout_id: identity.checkout_id.clone(),
            root: identity.root.clone(),
            normalized_root: identity.normalized_root.clone(),
            generation: AtomicU64::new(generation),
            core: WorkspaceCoreServices::new(&identity),
            services,
            leases: Arc::new(WorkspaceLeaseTracker::default()),
            definition_cache: Arc::new(
                crate::workspace_definition_registry::DefinitionCacheEntry::new(&identity),
            ),
        });
        runtime.services.detect(&runtime);
        runtime
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub fn checkout_id(&self) -> &CheckoutId {
        &self.checkout_id
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn normalized_root(&self) -> &str {
        &self.normalized_root
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn core(&self) -> &WorkspaceCoreServices {
        &self.core
    }

    pub fn knowledge_index(
        &self,
        app_handle: &tauri::AppHandle,
    ) -> Result<Arc<crate::knowledge_index::KnowledgeIndexState>, String> {
        self.core.knowledge_index(
            crate::workspace_service::event::WorkspaceEventScope::for_runtime(self),
            app_handle,
        )
    }

    pub fn services(&self) -> &Arc<WorkspaceServiceHost> {
        &self.services
    }

    pub fn lease_count(&self) -> usize {
        self.leases.count()
    }

    pub fn idle_for(&self) -> std::time::Duration {
        self.leases.idle_for()
    }

    pub fn acquire_lease(self: &Arc<Self>, kind: WorkspaceLeaseKind) -> WorkspaceLease {
        WorkspaceLease {
            _token: Arc::new(self.leases.acquire(kind)),
        }
    }

    pub fn activity_snapshot(
        &self,
        idle_timeout: std::time::Duration,
    ) -> WorkspaceActivitySnapshot {
        self.leases.snapshot(idle_timeout)
    }

    pub(crate) fn definition_cache_entry(
        &self,
    ) -> Arc<crate::workspace_definition_registry::DefinitionCacheEntry> {
        Arc::clone(&self.definition_cache)
    }
}

pub struct ProjectContext {
    project_id: ProjectId,
    sessions: ProjectSessionCatalog,
    knowledge: super::project_resources::ProjectKnowledgeCatalog,
    collaboration: super::project_resources::ProjectCollaborationHub,
    // ProjectContext is the ownership root for every checkout runtime. The
    // process-wide checkout table is only a weak lookup index.
    checkouts: RwLock<HashMap<CheckoutId, Arc<WorkspaceRuntime>>>,
}

impl ProjectContext {
    fn new(
        project_id: ProjectId,
        session_store: Arc<RwLock<Option<Arc<crate::session::store::SessionStore>>>>,
    ) -> Self {
        Self {
            sessions: ProjectSessionCatalog {
                project_id: project_id.clone(),
                store: session_store,
            },
            knowledge: super::project_resources::ProjectKnowledgeCatalog::new(project_id.clone()),
            collaboration: super::project_resources::ProjectCollaborationHub::new(
                project_id.clone(),
            ),
            project_id,
            checkouts: RwLock::new(HashMap::new()),
        }
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub fn sessions(&self) -> &ProjectSessionCatalog {
        &self.sessions
    }

    pub fn knowledge(&self) -> &super::project_resources::ProjectKnowledgeCatalog {
        &self.knowledge
    }

    pub fn collaboration(&self) -> &super::project_resources::ProjectCollaborationHub {
        &self.collaboration
    }

    pub fn checkout_ids(&self) -> Vec<CheckoutId> {
        self.checkouts
            .read()
            .map(|checkouts| checkouts.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn checkout(&self, checkout_id: &CheckoutId) -> Option<Arc<WorkspaceRuntime>> {
        self.checkouts
            .read()
            .ok()
            .and_then(|checkouts| checkouts.get(checkout_id).cloned())
    }

    pub fn runtimes(&self) -> Vec<Arc<WorkspaceRuntime>> {
        self.checkouts
            .read()
            .map(|checkouts| checkouts.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Build the project read model from durable checkout records. Runtime
    /// generation is an optional enrichment and never controls visibility.
    pub fn checkout_sources(
        &self,
    ) -> Result<Vec<super::project_resources::ProjectCheckoutSource>, String> {
        let records = self
            .sessions
            .store()?
            .list_workspace_checkouts(Some(self.project_id.as_str()))?;
        let runtimes = self
            .checkouts
            .read()
            .map_err(|error| format!("project checkout lock poisoned: {error}"))?;
        Ok(records
            .into_iter()
            .map(|record| {
                let workspace_generation = CheckoutId::new(record.checkout_id.clone())
                    .ok()
                    .and_then(|checkout_id| runtimes.get(&checkout_id))
                    .map(|runtime| runtime.generation());
                super::project_resources::ProjectCheckoutSource {
                    checkout_id: record.checkout_id,
                    root: PathBuf::from(record.root_path),
                    workspace_generation,
                }
            })
            .collect())
    }
}

/// Project-owned view over the durable session catalog. Session persistence
/// remains authoritative in `SessionStore`; ProjectContext supplies the
/// project boundary used by runtime callers.
pub struct ProjectSessionCatalog {
    project_id: ProjectId,
    store: Arc<RwLock<Option<Arc<crate::session::store::SessionStore>>>>,
}

impl ProjectSessionCatalog {
    fn store(&self) -> Result<Arc<crate::session::store::SessionStore>, String> {
        self.store
            .read()
            .map_err(|error| format!("project session catalog lock poisoned: {error}"))?
            .clone()
            .ok_or_else(|| "project session catalog is not attached".to_string())
    }

    pub fn list(&self) -> Result<Vec<crate::session::models::SessionSummary>, String> {
        self.store()?.list_sessions(Some(self.project_id.as_str()))
    }

    pub fn list_archived(&self) -> Result<Vec<crate::session::models::SessionSummary>, String> {
        self.store()?
            .list_archived_sessions(Some(self.project_id.as_str()))
    }

    pub fn list_for_checkout(
        &self,
        checkout_id: &CheckoutId,
    ) -> Result<Vec<crate::session::models::SessionSummary>, String> {
        let sessions = self
            .store()?
            .list_sessions_for_checkout(checkout_id.as_str())?;
        if sessions
            .iter()
            .any(|session| session.project_id.as_deref() != Some(self.project_id.as_str()))
        {
            return Err(format!(
                "checkout {} returned a session outside project {}",
                checkout_id, self.project_id
            ));
        }
        Ok(sessions)
    }

    pub fn list_archived_for_checkout(
        &self,
        checkout_id: &CheckoutId,
    ) -> Result<Vec<crate::session::models::SessionSummary>, String> {
        let sessions = self
            .store()?
            .list_archived_sessions_for_checkout(checkout_id.as_str())?;
        if sessions
            .iter()
            .any(|session| session.project_id.as_deref() != Some(self.project_id.as_str()))
        {
            return Err(format!(
                "checkout {} returned an archived session outside project {}",
                checkout_id, self.project_id
            ));
        }
        Ok(sessions)
    }

    /// Resolve a project-owned session for an execution checkout. Sessions are
    /// shared by sibling worktrees; the checkout only supplies the run target.
    pub fn resolve_for_checkout(
        &self,
        checkout_id: &CheckoutId,
        session_id: &str,
    ) -> Result<crate::session::models::SessionWorkspaceScope, String> {
        let scope = self.store()?.get_session_workspace_scope(session_id)?;
        if scope.project_id.as_deref() != Some(self.project_id.as_str()) {
            return Err(format!(
                "session {session_id} is outside project {}",
                self.project_id
            ));
        }
        let checkout = self
            .store()?
            .get_workspace_checkout(checkout_id.as_str())?
            .ok_or_else(|| format!("unknown project checkout {checkout_id}"))?;
        if checkout.project_id != self.project_id.as_str() {
            return Err(format!(
                "checkout {checkout_id} belongs to project {}, not {}",
                checkout.project_id, self.project_id
            ));
        }
        Ok(scope)
    }
}

struct ServiceStartGate {
    active: Mutex<usize>,
    waiting: AtomicUsize,
    notify: Notify,
}

impl Default for ServiceStartGate {
    fn default() -> Self {
        Self {
            active: Mutex::new(0),
            waiting: AtomicUsize::new(0),
            notify: Notify::new(),
        }
    }
}

impl ServiceStartGate {
    async fn acquire(self: &Arc<Self>, policy: &ResourcePolicyStore) -> ServiceStartPermit {
        let mut wait_registration = ServiceStartWaitRegistration {
            gate: Arc::clone(self),
            armed: false,
        };
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let limit = policy.snapshot().limits.max_concurrent_service_starts;
            {
                let mut active = self.active.lock().expect("service start gate lock");
                if *active < limit {
                    *active += 1;
                    wait_registration.disarm();
                    return ServiceStartPermit {
                        gate: Arc::clone(self),
                    };
                }
            }
            wait_registration.arm();
            notified.await;
        }
    }

    fn policy_changed(&self) {
        self.notify.notify_waiters();
    }

    fn snapshot(&self) -> (usize, usize) {
        (
            self.active.lock().map(|active| *active).unwrap_or(0),
            self.waiting.load(Ordering::Acquire),
        )
    }
}

struct ServiceStartWaitRegistration {
    gate: Arc<ServiceStartGate>,
    armed: bool,
}

impl ServiceStartWaitRegistration {
    fn arm(&mut self) {
        if !self.armed {
            self.gate.waiting.fetch_add(1, Ordering::AcqRel);
            self.armed = true;
        }
    }

    fn disarm(&mut self) {
        if self.armed {
            self.gate.waiting.fetch_sub(1, Ordering::AcqRel);
            self.armed = false;
        }
    }
}

impl Drop for ServiceStartWaitRegistration {
    fn drop(&mut self) {
        self.disarm();
    }
}

struct ServiceStartPermit {
    gate: Arc<ServiceStartGate>,
}

impl Drop for ServiceStartPermit {
    fn drop(&mut self) {
        if let Ok(mut active) = self.gate.active.lock() {
            *active = active.saturating_sub(1);
        }
        self.gate.notify.notify_waiters();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ServiceAdmissionKey {
    checkout_id: CheckoutId,
    kind: ServiceKind,
}

#[derive(Default)]
struct ServiceAdmissionState {
    occupied: HashSet<ServiceAdmissionKey>,
    operation_locks: HashMap<ServiceAdmissionKey, Arc<tokio::sync::Mutex<()>>>,
}

#[derive(Default)]
struct ServiceAdmissionCoordinator {
    state: Mutex<ServiceAdmissionState>,
}

impl ServiceAdmissionCoordinator {
    async fn lock_operation(&self, key: &ServiceAdmissionKey) -> tokio::sync::OwnedMutexGuard<()> {
        let operation_lock = {
            let mut state = self.state.lock().expect("service admission lock");
            Arc::clone(
                state
                    .operation_locks
                    .entry(key.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        operation_lock.lock_owned().await
    }

    fn reserve(
        self: &Arc<Self>,
        key: ServiceAdmissionKey,
        limit: usize,
    ) -> Result<ServiceCapacityReservation, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("service admission lock poisoned: {error}"))?;
        if !state.occupied.contains(&key) && state.occupied.len() >= limit {
            return Err(format!(
                "workspace service capacity is busy (configured limit: {limit})"
            ));
        }
        state.occupied.insert(key.clone());
        Ok(ServiceCapacityReservation {
            coordinator: Arc::clone(self),
            key,
            committed: false,
        })
    }

    fn observe_running(&self, key: ServiceAdmissionKey) {
        if let Ok(mut state) = self.state.lock() {
            state.occupied.insert(key);
        }
    }

    fn release(&self, key: &ServiceAdmissionKey) {
        if let Ok(mut state) = self.state.lock() {
            state.occupied.remove(key);
        }
    }

    fn remove_checkout(&self, checkout_id: &CheckoutId) {
        if let Ok(mut state) = self.state.lock() {
            state.occupied.retain(|key| &key.checkout_id != checkout_id);
            state
                .operation_locks
                .retain(|key, _| &key.checkout_id != checkout_id);
        }
    }
}

struct ServiceCapacityReservation {
    coordinator: Arc<ServiceAdmissionCoordinator>,
    key: ServiceAdmissionKey,
    committed: bool,
}

impl ServiceCapacityReservation {
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for ServiceCapacityReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.coordinator.release(&self.key);
        }
    }
}

static NEXT_WORKSPACE_RUNTIME_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_workspace_runtime_generation() -> u64 {
    NEXT_WORKSPACE_RUNTIME_GENERATION
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            current.checked_add(1)
        })
        .expect("workspace runtime generation space exhausted")
}

pub struct ProjectRegistry {
    projects: RwLock<HashMap<ProjectId, Arc<ProjectContext>>>,
    // Weak, process-wide lookup index. Strong runtime ownership lives in the
    // matching ProjectContext.checkouts hierarchy.
    checkouts: RwLock<HashMap<CheckoutId, Weak<WorkspaceRuntime>>>,
    registration_gates: Mutex<HashMap<CheckoutId, Arc<Mutex<()>>>>,
    session_store: Arc<RwLock<Option<Arc<crate::session::store::SessionStore>>>>,
    resource_policy: Arc<ResourcePolicyStore>,
    factories: Vec<Arc<dyn WorkspaceServiceFactory>>,
    service_start_gate: Arc<ServiceStartGate>,
    service_admission: Arc<ServiceAdmissionCoordinator>,
    event_router: Arc<WorkspaceEventRouter>,
    runtime_registration_hooks: Mutex<Vec<RuntimeRegistrationHook>>,
    runtime_retirement_hooks: Mutex<Vec<RuntimeRetirementHook>>,
}

type RuntimeRegistrationHook =
    Arc<dyn Fn(&Arc<WorkspaceRuntime>) -> Result<(), String> + Send + Sync + 'static>;
type RuntimeRetirementHook = Arc<dyn Fn(&WorkspaceRuntime) + Send + Sync + 'static>;

impl ProjectRegistry {
    pub fn new(
        resource_policy: Arc<ResourcePolicyStore>,
        factories: Vec<Arc<dyn WorkspaceServiceFactory>>,
    ) -> Arc<Self> {
        let registry = Arc::new(Self {
            projects: RwLock::new(HashMap::new()),
            checkouts: RwLock::new(HashMap::new()),
            registration_gates: Mutex::new(HashMap::new()),
            session_store: Arc::new(RwLock::new(None)),
            resource_policy,
            factories,
            service_start_gate: Arc::new(ServiceStartGate::default()),
            service_admission: Arc::new(ServiceAdmissionCoordinator::default()),
            event_router: WorkspaceEventRouter::new(),
            runtime_registration_hooks: Mutex::new(Vec::new()),
            runtime_retirement_hooks: Mutex::new(Vec::new()),
        });
        registry.event_router.attach_registry(&registry);
        registry
    }

    pub fn resource_policy(&self) -> &Arc<ResourcePolicyStore> {
        &self.resource_policy
    }

    pub fn event_router(&self) -> &Arc<WorkspaceEventRouter> {
        &self.event_router
    }

    pub fn attach_session_store(
        &self,
        store: &Arc<crate::session::store::SessionStore>,
    ) -> Result<(), String> {
        let persisted_checkouts = store.list_workspace_checkouts(None)?;
        *self
            .session_store
            .write()
            .map_err(|error| format!("project session catalog lock poisoned: {error}"))? =
            Some(Arc::clone(store));
        let mut projects = self
            .projects
            .write()
            .map_err(|error| format!("project registry write lock poisoned: {error}"))?;
        for checkout in persisted_checkouts {
            let project_id = ProjectId::new(checkout.project_id).map_err(|error| {
                format!("persisted workspace project identity is invalid: {error}")
            })?;
            projects.entry(project_id.clone()).or_insert_with(|| {
                Arc::new(ProjectContext::new(
                    project_id,
                    Arc::clone(&self.session_store),
                ))
            });
        }
        Ok(())
    }

    /// Ensure a durable checkout has a live runtime. This is the silent
    /// activation boundary used when a user opens a persisted session.
    pub fn activate_persisted_checkout(
        &self,
        checkout_id: &CheckoutId,
    ) -> Result<Arc<WorkspaceRuntime>, String> {
        if let Some(runtime) = self.runtime(checkout_id) {
            self.ensure_runtime_initialized(&runtime)?;
            return Ok(runtime);
        }
        let store = self
            .session_store
            .read()
            .map_err(|error| format!("project session catalog lock poisoned: {error}"))?
            .clone()
            .ok_or_else(|| "project session catalog is not attached".to_string())?;
        let checkout = store
            .get_workspace_checkout(checkout_id.as_str())?
            .ok_or_else(|| format!("unknown persisted checkout {checkout_id}"))?;
        let runtime = self.register(&checkout.root_path)?;
        if runtime.checkout_id() != checkout_id {
            return Err(format!(
                "persisted checkout identity changed: expected {checkout_id}, resolved {}",
                runtime.checkout_id()
            ));
        }
        if runtime.project_id().as_str() != checkout.project_id {
            return Err(format!(
                "persisted checkout project changed: expected {}, resolved {}",
                checkout.project_id,
                runtime.project_id()
            ));
        }
        Ok(runtime)
    }

    pub(crate) fn add_runtime_retirement_hook(
        &self,
        hook: RuntimeRetirementHook,
    ) -> Result<(), String> {
        self.runtime_retirement_hooks
            .lock()
            .map_err(|error| format!("runtime retirement hook lock poisoned: {error}"))?
            .push(hook);
        Ok(())
    }

    pub(crate) fn add_runtime_registration_hook(
        &self,
        hook: RuntimeRegistrationHook,
    ) -> Result<(), String> {
        self.runtime_registration_hooks
            .lock()
            .map_err(|error| format!("runtime registration hook lock poisoned: {error}"))?
            .push(hook);
        Ok(())
    }

    pub(crate) fn ensure_runtime_initialized(
        &self,
        runtime: &Arc<WorkspaceRuntime>,
    ) -> Result<(), String> {
        let hooks = self
            .runtime_registration_hooks
            .lock()
            .map_err(|error| format!("runtime registration hook lock poisoned: {error}"))?
            .clone();
        for hook in hooks {
            hook(runtime)?;
        }
        Ok(())
    }

    pub fn register(&self, path: impl AsRef<Path>) -> Result<Arc<WorkspaceRuntime>, String> {
        let identity = ProjectIdResolver::resolve(path).map_err(|error| error.to_string())?;
        // Runtime construction performs service detection and may open
        // checkout-owned stores. Every path, including an idempotent lookup,
        // enters the per-checkout gate so registration cannot observe a
        // generation while its predecessor is still retiring.
        let registration_gate = {
            let mut gates = self
                .registration_gates
                .lock()
                .map_err(|error| format!("workspace registration gates lock poisoned: {error}"))?;
            Arc::clone(
                gates
                    .entry(identity.checkout_id.clone())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _registration = registration_gate
            .lock()
            .map_err(|error| format!("workspace registration lock poisoned: {error}"))?;
        if let Some(existing) = self
            .checkouts
            .read()
            .map_err(|error| format!("workspace registry read lock poisoned: {error}"))?
            .get(&identity.checkout_id)
            .and_then(Weak::upgrade)
        {
            self.ensure_runtime_initialized(&existing)?;
            return Ok(existing);
        }

        let runtime = WorkspaceRuntime::new(
            identity,
            self.factories.clone(),
            next_workspace_runtime_generation(),
        );
        if let Err(error) = self.ensure_runtime_initialized(&runtime) {
            runtime.core().stop_background_watchers();
            return Err(error);
        }
        let project = {
            let mut projects = self
                .projects
                .write()
                .map_err(|error| format!("project registry write lock poisoned: {error}"))?;
            projects
                .entry(runtime.project_id().clone())
                .or_insert_with(|| {
                    Arc::new(ProjectContext::new(
                        runtime.project_id().clone(),
                        Arc::clone(&self.session_store),
                    ))
                })
                .clone()
        };
        let mut project_checkouts = project
            .checkouts
            .write()
            .map_err(|error| format!("project checkout write lock poisoned: {error}"))?;
        let mut checkouts = self
            .checkouts
            .write()
            .map_err(|error| format!("workspace registry write lock poisoned: {error}"))?;
        if let Some(existing) = checkouts.get(runtime.checkout_id()).and_then(Weak::upgrade) {
            project_checkouts.insert(existing.checkout_id().clone(), Arc::clone(&existing));
            drop(checkouts);
            drop(project_checkouts);
            runtime.core().stop_background_watchers();
            self.ensure_runtime_initialized(&existing)?;
            return Ok(existing);
        }

        // Publish bottom-up while holding both hierarchy locks: once the weak
        // process-wide index can resolve a runtime, its ProjectContext already
        // owns the strong Arc.
        project_checkouts.insert(runtime.checkout_id().clone(), Arc::clone(&runtime));
        checkouts.insert(runtime.checkout_id().clone(), Arc::downgrade(&runtime));
        Ok(runtime)
    }

    pub fn open_workspace(&self, path: impl AsRef<Path>) -> Result<Arc<WorkspaceRuntime>, String> {
        self.register(path)
    }

    pub fn runtime(&self, checkout_id: &CheckoutId) -> Option<Arc<WorkspaceRuntime>> {
        self.checkouts
            .read()
            .ok()
            .and_then(|checkouts| checkouts.get(checkout_id).and_then(Weak::upgrade))
    }

    /// Resolve a request scope and acquire its runtime lease atomically with
    /// the registry lookup. A supplied generation always refers to the runtime
    /// incarnation.
    pub fn resolve_workspace_ref(
        &self,
        workspace_ref: &WorkspaceRef,
    ) -> Result<ResolvedWorkspaceScope, WorkspaceResolveError> {
        let checkouts =
            self.checkouts
                .read()
                .map_err(|error| WorkspaceResolveError::RegistryUnavailable {
                    detail: error.to_string(),
                })?;
        let runtime = checkouts
            .get(&workspace_ref.checkout_id)
            .and_then(Weak::upgrade)
            .ok_or_else(|| WorkspaceResolveError::CheckoutUnavailable {
                checkout_id: workspace_ref.checkout_id.clone(),
            })?;
        let actual_generation = runtime.generation();
        if let Some(expected_generation) = workspace_ref.expected_generation {
            if expected_generation != actual_generation {
                return Err(WorkspaceResolveError::StaleGeneration {
                    checkout_id: workspace_ref.checkout_id.clone(),
                    expected_generation,
                    actual_generation,
                });
            }
        }
        let lease = runtime.acquire_lease(WorkspaceLeaseKind::RunningTask);
        Ok(ResolvedWorkspaceScope::new(runtime, lease))
    }

    pub fn runtime_for_root(&self, root: &Path) -> Option<Arc<WorkspaceRuntime>> {
        let identity = ProjectIdResolver::resolve(root).ok()?;
        self.runtime(&identity.checkout_id)
    }

    pub fn project(&self, project_id: &ProjectId) -> Option<Arc<ProjectContext>> {
        self.projects
            .read()
            .ok()
            .and_then(|projects| projects.get(project_id).cloned())
    }

    pub fn checkout_count(&self) -> usize {
        self.checkouts
            .read()
            .map(|items| {
                items
                    .values()
                    .filter(|runtime| runtime.strong_count() > 0)
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn runtimes(&self) -> Vec<Arc<WorkspaceRuntime>> {
        self.checkouts
            .read()
            .map(|checkouts| checkouts.values().filter_map(Weak::upgrade).collect())
            .unwrap_or_default()
    }

    fn runtime_with_lease(
        &self,
        checkout_id: &CheckoutId,
    ) -> Result<(Arc<WorkspaceRuntime>, WorkspaceLease), String> {
        self.resolve_workspace_ref(&WorkspaceRef::new(checkout_id.clone(), None))
            .map(ResolvedWorkspaceScope::into_parts)
            .map_err(|error| error.to_string())
    }

    fn service_key(runtime: &WorkspaceRuntime, kind: ServiceKind) -> ServiceAdmissionKey {
        ServiceAdmissionKey {
            checkout_id: runtime.checkout_id().clone(),
            kind,
        }
    }

    async fn stop_service(
        &self,
        runtime: &Arc<WorkspaceRuntime>,
        kind: ServiceKind,
    ) -> Result<(), String> {
        let key = Self::service_key(runtime, kind);
        let _operation = self.service_admission.lock_operation(&key).await;
        runtime.services().stop(kind).await?;
        self.service_admission.release(&key);
        Ok(())
    }

    pub async fn execution_context(
        &self,
        checkout_id: &CheckoutId,
        requested_services: &[ServiceKind],
    ) -> Result<Arc<AgentExecutionContext>, String> {
        let (workspace, acquisition_lease) = self.runtime_with_lease(checkout_id)?;
        let mut service_bindings = HashMap::<ServiceKind, ServiceBinding>::new();
        for kind in requested_services {
            let key = Self::service_key(&workspace, *kind);
            let _operation = self.service_admission.lock_operation(&key).await;
            if workspace.services().is_running(*kind).await {
                self.service_admission.observe_running(key);
                let binding = workspace
                    .services()
                    .bind(Arc::clone(&workspace), *kind, true)
                    .await?;
                service_bindings.insert(*kind, binding);
                continue;
            }

            let max_running = self
                .resource_policy
                .snapshot()
                .limits
                .max_running_workspace_services;
            let reservation = self.service_admission.reserve(key, max_running)?;
            let _start_permit = self
                .service_start_gate
                .clone()
                .acquire(&self.resource_policy)
                .await;
            let binding = workspace
                .services()
                .bind(Arc::clone(&workspace), *kind, true)
                .await?;
            reservation.commit();
            service_bindings.insert(*kind, binding);
        }
        let execution = Arc::new(AgentExecutionContext::new(workspace, service_bindings));
        drop(acquisition_lease);
        Ok(execution)
    }

    pub async fn shutdown_all(&self) {
        for runtime in self.runtimes() {
            for kind in runtime.services().detected_kinds() {
                if let Err(error) = self.stop_service(&runtime, kind).await {
                    eprintln!(
                        "[Locus] failed to stop {} service for checkout {} during exit: {}",
                        kind.as_str(),
                        runtime.checkout_id(),
                        error
                    );
                }
            }
            runtime.core().stop_background_watchers();
        }
    }

    pub async fn tool_execution_context(
        &self,
        root: &Path,
        tool_name: &str,
    ) -> Result<Arc<AgentExecutionContext>, String> {
        let runtime = self.register(root)?;
        let requested_services = super::service::owner_service_for_tool(tool_name)
            .into_iter()
            .collect::<Vec<_>>();
        self.execution_context(runtime.checkout_id(), &requested_services)
            .await
    }

    pub fn notify_policy_changed(&self) {
        self.service_start_gate.policy_changed();
    }

    pub fn spawn_idle_reaper(self: &Arc<Self>) {
        let registry = Arc::clone(self);
        let mut policy_updates = self.resource_policy.subscribe();
        tauri::async_runtime::spawn(async move {
            loop {
                let limits = policy_updates.borrow().limits.clone();
                let next_check_secs = limits
                    .workspace_idle_timeout_secs
                    .min(limits.service_idle_timeout_secs);
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(next_check_secs)) => {
                        registry.reap_idle_resources().await;
                    }
                    changed = policy_updates.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        registry.converge_resource_policy().await;
                    }
                }
            }
        });
    }

    pub async fn converge_resource_policy(&self) {
        let limits = self.resource_policy.snapshot().limits;
        let activity_idle_timeout =
            std::time::Duration::from_secs(limits.workspace_idle_timeout_secs);
        let runtimes = self.runtimes();
        let mut running = Vec::new();
        for runtime in &runtimes {
            let activity = runtime.activity_snapshot(activity_idle_timeout);
            for (kind, _, leases, idle_for) in runtime.services().running_snapshot().await {
                running.push((Arc::clone(runtime), kind, leases, idle_for, activity));
            }
        }
        running.sort_by_key(|(runtime, _, _, idle_for, activity)| {
            (
                activity.priority,
                std::cmp::Reverse(*idle_for),
                runtime.checkout_id().to_string(),
            )
        });
        let mut running_count = running.len();
        for (runtime, kind, leases, _, activity) in running {
            if running_count <= limits.max_running_workspace_services {
                break;
            }
            if leases == 0
                && !activity.priority.protects_resources()
                && !runtime
                    .activity_snapshot(activity_idle_timeout)
                    .priority
                    .protects_resources()
                && self.stop_service(&runtime, kind).await.is_ok()
            {
                running_count = running_count.saturating_sub(1);
            }
        }
        self.converge_background_watcher_policy(activity_idle_timeout);
    }

    fn converge_background_watcher_policy(&self, activity_idle_timeout: std::time::Duration) {
        let max_watched = self
            .resource_policy
            .snapshot()
            .limits
            .max_watched_workspaces;
        let mut watched = self
            .runtimes()
            .into_iter()
            .filter(|runtime| runtime.core().watchers_running())
            .collect::<Vec<_>>();
        watched.sort_by_key(|runtime| {
            let activity = runtime.activity_snapshot(activity_idle_timeout);
            (
                activity.priority,
                std::cmp::Reverse(activity.idle_for),
                runtime.checkout_id().to_string(),
            )
        });
        let mut count = watched.len();
        for runtime in watched {
            if count <= max_watched {
                break;
            }
            if !runtime
                .activity_snapshot(activity_idle_timeout)
                .priority
                .protects_resources()
            {
                runtime.core().stop_background_watchers();
                count = count.saturating_sub(1);
            }
        }
    }

    async fn reap_idle_resources(&self) {
        let limits = self.resource_policy.snapshot().limits;
        let activity_idle_timeout =
            std::time::Duration::from_secs(limits.workspace_idle_timeout_secs);
        for runtime in self.runtimes() {
            let activity = runtime.activity_snapshot(activity_idle_timeout);
            for (kind, _, leases, idle_for) in runtime.services().running_snapshot().await {
                if leases == 0
                    && !activity.priority.protects_resources()
                    && idle_for >= std::time::Duration::from_secs(limits.service_idle_timeout_secs)
                    && !runtime
                        .activity_snapshot(activity_idle_timeout)
                        .priority
                        .protects_resources()
                {
                    let _ = self.stop_service(&runtime, kind).await;
                }
            }
        }

        let candidates = self
            .runtimes()
            .into_iter()
            .filter(|runtime| {
                runtime
                    .activity_snapshot(activity_idle_timeout)
                    .priority
                    .is_idle()
            })
            .collect::<Vec<_>>();
        for runtime in candidates {
            // The checkout runtime is the stable process-lifetime address for
            // workspace state. Retire its optional data-plane resources while
            // preserving the runtime identity and generation.
            runtime.core().stop_background_watchers();
        }
    }

    fn remove_runtime_if_unleased(&self, candidate: &Arc<WorkspaceRuntime>) -> bool {
        let registration_gate = {
            let Ok(mut gates) = self.registration_gates.lock() else {
                return false;
            };
            Arc::clone(
                gates
                    .entry(candidate.checkout_id().clone())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let Ok(registration) = registration_gate.lock() else {
            return false;
        };
        let Some(project) = self.project(candidate.project_id()) else {
            return false;
        };

        // Match register's hierarchy -> index lock order. Removing both
        // references in one critical section prevents a lookup from seeing a
        // runtime after its project owner has released it.
        let removed = {
            let Ok(mut project_checkouts) = project.checkouts.write() else {
                return false;
            };
            let Ok(mut checkouts) = self.checkouts.write() else {
                return false;
            };
            let Some(project_runtime) = project_checkouts.get(candidate.checkout_id()) else {
                return false;
            };
            let Some(index_runtime) = checkouts
                .get(candidate.checkout_id())
                .and_then(Weak::upgrade)
            else {
                return false;
            };
            if project_runtime.generation() != candidate.generation()
                || index_runtime.generation() != candidate.generation()
                || project_runtime.lease_count() > 0
            {
                return false;
            }
            checkouts.remove(candidate.checkout_id());
            project_checkouts.remove(candidate.checkout_id())
        };
        let Some(removed) = removed else {
            return false;
        };

        removed.core().stop_background_watchers();
        if let Ok(hooks) = self.runtime_retirement_hooks.lock() {
            for hook in hooks.iter() {
                hook(removed.as_ref());
            }
        }
        self.service_admission
            .remove_checkout(removed.checkout_id());
        drop(registration);
        drop(registration_gate);
        if let Ok(mut gates) = self.registration_gates.lock() {
            let gate_is_idle = gates
                .get(removed.checkout_id())
                .is_some_and(|gate| Arc::strong_count(gate) == 1);
            if gate_is_idle {
                gates.remove(removed.checkout_id());
            }
        }
        true
    }

    pub async fn metrics(&self) -> WorkspaceRegistryMetrics {
        let limits = self.resource_policy.snapshot().limits;
        let runtimes = self
            .checkouts
            .read()
            .map(|checkouts| {
                checkouts
                    .values()
                    .filter_map(Weak::upgrade)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut running_services = 0;
        let mut service_leases = 0;
        let mut workspace_leases = 0;
        let mut watched_workspaces = 0;
        let mut running_task_workspaces = 0;
        let mut visible_pane_workspaces = 0;
        let mut background_open_workspaces = 0;
        let mut idle_workspaces = 0;
        let activity_idle_timeout =
            std::time::Duration::from_secs(limits.workspace_idle_timeout_secs);
        for runtime in runtimes {
            if runtime.core().watchers_running() {
                watched_workspaces += 1;
            }
            workspace_leases += runtime.lease_count();
            match runtime.activity_snapshot(activity_idle_timeout).priority {
                WorkspaceActivityPriority::RunningTask => running_task_workspaces += 1,
                WorkspaceActivityPriority::VisiblePane => visible_pane_workspaces += 1,
                WorkspaceActivityPriority::BackgroundOpen => background_open_workspaces += 1,
                WorkspaceActivityPriority::Idle => idle_workspaces += 1,
            }
            for (_, _, leases, _) in runtime.services().running_snapshot().await {
                running_services += 1;
                service_leases += leases;
            }
        }
        let (service_starts_active, service_starts_waiting) = self.service_start_gate.snapshot();
        WorkspaceRegistryMetrics {
            configured_max_running_workspace_services: limits.max_running_workspace_services,
            configured_max_concurrent_service_starts: limits.max_concurrent_service_starts,
            configured_max_watched_workspaces: limits.max_watched_workspaces,
            registered_workspace_runtimes: self.checkout_count(),
            running_workspace_services: running_services,
            watched_workspaces,
            service_starts_active,
            service_starts_waiting,
            workspace_leases,
            service_leases,
            running_task_workspaces,
            visible_pane_workspaces,
            background_open_workspaces,
            idle_workspaces,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRegistryMetrics {
    pub configured_max_running_workspace_services: usize,
    pub configured_max_concurrent_service_starts: usize,
    pub configured_max_watched_workspaces: usize,
    pub registered_workspace_runtimes: usize,
    pub running_workspace_services: usize,
    pub watched_workspaces: usize,
    pub service_starts_active: usize,
    pub service_starts_waiting: usize,
    pub workspace_leases: usize,
    pub service_leases: usize,
    pub running_task_workspaces: usize,
    pub visible_pane_workspaces: usize,
    pub background_open_workspaces: usize,
    pub idle_workspaces: usize,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::Duration;

    use super::*;
    use crate::config::{AppConfig, WorkspaceServiceResourceLimits};
    use crate::workspace_service::service::{
        DetectionResult, PromptFragment, ServiceActivationPolicy, ServiceBindingError,
        ServiceCapabilities, ServiceContextProvider, ServiceFuture, ServiceLeaseTracker,
        ServiceRuntimeIdentity, ServiceStatus, ServiceToolDefinition, ServiceToolProvider,
        WorkspaceService,
    };
    use crate::workspace_service::{ServiceInstanceId, WindowContextRegistry};

    struct FakeStartControl {
        block_starts: AtomicBool,
        entered: tokio::sync::Semaphore,
        release: tokio::sync::Semaphore,
    }

    impl Default for FakeStartControl {
        fn default() -> Self {
            Self {
                block_starts: AtomicBool::new(false),
                entered: tokio::sync::Semaphore::new(0),
                release: tokio::sync::Semaphore::new(0),
            }
        }
    }

    impl FakeStartControl {
        fn blocked() -> Arc<Self> {
            Arc::new(Self {
                block_starts: AtomicBool::new(true),
                ..Self::default()
            })
        }

        async fn wait_until_entered(&self) {
            let permit = self.entered.acquire().await.expect("start-entry semaphore");
            permit.forget();
        }

        fn release_one(&self) {
            self.release.add_permits(1);
        }
    }

    struct FakeServiceFactory {
        detects: AtomicUsize,
        creates: AtomicUsize,
        starts: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
        control: Arc<FakeStartControl>,
    }

    impl FakeServiceFactory {
        fn immediate() -> Arc<Self> {
            Arc::new(Self {
                detects: AtomicUsize::new(0),
                creates: AtomicUsize::new(0),
                starts: Arc::new(AtomicUsize::new(0)),
                stops: Arc::new(AtomicUsize::new(0)),
                control: Arc::new(FakeStartControl::default()),
            })
        }

        fn blocked() -> Arc<Self> {
            Arc::new(Self {
                detects: AtomicUsize::new(0),
                creates: AtomicUsize::new(0),
                starts: Arc::new(AtomicUsize::new(0)),
                stops: Arc::new(AtomicUsize::new(0)),
                control: FakeStartControl::blocked(),
            })
        }
    }

    impl WorkspaceServiceFactory for FakeServiceFactory {
        fn kind(&self) -> ServiceKind {
            ServiceKind::Unity
        }

        fn detect(&self, _workspace: &WorkspaceRuntime) -> DetectionResult {
            self.detects.fetch_add(1, Ordering::SeqCst);
            DetectionResult::detected(ServiceActivationPolicy::Lazy)
        }

        fn create<'a>(
            &'a self,
            workspace: Arc<WorkspaceRuntime>,
            generation: u64,
        ) -> ServiceFuture<'a, Result<Arc<dyn WorkspaceService>, String>> {
            self.creates.fetch_add(1, Ordering::SeqCst);
            let service: Arc<dyn WorkspaceService> = Arc::new(FakeService {
                identity: ServiceRuntimeIdentity {
                    project_id: workspace.project_id().clone(),
                    checkout_id: workspace.checkout_id().clone(),
                    service_instance_id: ServiceInstanceId::for_service(
                        workspace.checkout_id(),
                        ServiceKind::Unity.as_str(),
                    ),
                    runtime_generation: generation,
                },
                status: Mutex::new(ServiceStatus::Dormant),
                leases: Arc::new(ServiceLeaseTracker::default()),
                starts: Arc::clone(&self.starts),
                stops: Arc::clone(&self.stops),
                control: Arc::clone(&self.control),
            });
            Box::pin(async move { Ok(service) })
        }
    }

    struct FakeService {
        identity: ServiceRuntimeIdentity,
        status: Mutex<ServiceStatus>,
        leases: Arc<ServiceLeaseTracker>,
        starts: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
        control: Arc<FakeStartControl>,
    }

    impl FakeService {
        fn set_status(&self, status: ServiceStatus) {
            *self.status.lock().expect("fake service status") = status;
        }
    }

    impl WorkspaceService for FakeService {
        fn identity(&self) -> ServiceRuntimeIdentity {
            self.identity.clone()
        }

        fn status(&self) -> ServiceStatus {
            *self.status.lock().expect("fake service status")
        }

        fn capabilities(&self) -> ServiceCapabilities {
            ServiceCapabilities::default()
        }

        fn lease_tracker(&self) -> Arc<ServiceLeaseTracker> {
            Arc::clone(&self.leases)
        }

        fn start(&self) -> ServiceFuture<'_, Result<(), String>> {
            Box::pin(async move {
                self.starts.fetch_add(1, Ordering::SeqCst);
                self.set_status(ServiceStatus::Starting);
                if self.control.block_starts.load(Ordering::Acquire) {
                    self.control.entered.add_permits(1);
                    let permit = self
                        .control
                        .release
                        .acquire()
                        .await
                        .map_err(|_| "start release semaphore closed".to_string())?;
                    permit.forget();
                }
                self.set_status(ServiceStatus::Running);
                Ok(())
            })
        }

        fn suspend(&self) -> ServiceFuture<'_, Result<(), String>> {
            Box::pin(async move {
                self.set_status(ServiceStatus::Dormant);
                Ok(())
            })
        }

        fn stop(&self) -> ServiceFuture<'_, Result<(), String>> {
            Box::pin(async move {
                self.stops.fetch_add(1, Ordering::SeqCst);
                self.set_status(ServiceStatus::Stopped);
                Ok(())
            })
        }

        fn tool_provider(&self) -> Arc<dyn ServiceToolProvider> {
            Arc::new(EmptyProvider)
        }

        fn context_provider(&self) -> Arc<dyn ServiceContextProvider> {
            Arc::new(EmptyProvider)
        }
    }

    struct EmptyProvider;

    impl ServiceToolProvider for EmptyProvider {
        fn tool_definitions(&self) -> Vec<ServiceToolDefinition> {
            Vec::new()
        }
    }

    impl ServiceContextProvider for EmptyProvider {
        fn prompt_fragments(&self, _execution: &AgentExecutionContext) -> Vec<PromptFragment> {
            Vec::new()
        }
    }

    fn registry(
        config_dir: &Path,
        factory: Arc<FakeServiceFactory>,
        max_running: usize,
    ) -> Arc<ProjectRegistry> {
        let config = Arc::new(AppConfig::load_from_path(&config_dir.join("config.json")));
        let policy = Arc::new(ResourcePolicyStore::from_config(config).expect("resource policy"));
        let mut limits = WorkspaceServiceResourceLimits::default();
        limits.max_running_workspace_services = max_running;
        limits.max_concurrent_service_starts = max_running.max(1);
        policy.update(limits).expect("update resource policy");
        let factory: Arc<dyn WorkspaceServiceFactory> = factory;
        ProjectRegistry::new(policy, vec![factory])
    }

    #[test]
    fn project_context_catalog_groups_sessions_and_keeps_checkout_identity() {
        let temp = tempfile::tempdir().expect("temp root");
        let root_a = temp.path().join("checkout-a");
        let root_b = temp.path().join("checkout-b");
        for root in [&root_a, &root_b] {
            std::fs::create_dir_all(root.join("Locus")).expect("workspace config dir");
            std::fs::write(
                root.join("Locus/config.json"),
                r#"{"workspace_id":"shared-project"}"#,
            )
            .expect("workspace config");
        }
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 4);
        let runtime_a = registry.register(&root_a).expect("checkout A");
        let runtime_b = registry.register(&root_b).expect("checkout B");
        assert_eq!(runtime_a.project_id(), runtime_b.project_id());

        let store_dir = tempfile::tempdir().expect("session store");
        let store = Arc::new(
            crate::session::store::SessionStore::new(store_dir.path()).expect("session store"),
        );
        registry
            .attach_session_store(&store)
            .expect("attach session store");
        for runtime in [&runtime_a, &runtime_b] {
            store
                .upsert_workspace_checkout(&crate::session::models::WorkspaceCheckoutRecord {
                    checkout_id: runtime.checkout_id().to_string(),
                    project_id: runtime.project_id().to_string(),
                    root_path: runtime.root().display().to_string(),
                    normalized_root: runtime.normalized_root().to_string(),
                    last_opened_at: 1,
                })
                .expect("persist checkout");
        }
        let session_a = store
            .create_session_scoped(
                "A",
                None,
                Some(runtime_a.project_id().as_str()),
                Some(runtime_a.checkout_id().as_str()),
                "chat",
                None,
            )
            .expect("session A");
        store
            .create_session_scoped(
                "B",
                None,
                Some(runtime_b.project_id().as_str()),
                Some(runtime_b.checkout_id().as_str()),
                "chat",
                None,
            )
            .expect("session B");

        let project = registry
            .project(runtime_a.project_id())
            .expect("project context");
        assert_eq!(
            project.sessions().list().expect("project sessions").len(),
            2
        );
        assert_eq!(
            project
                .sessions()
                .list_for_checkout(runtime_a.checkout_id())
                .expect("checkout A sessions")
                .len(),
            1
        );
        store.archive_session(&session_a).expect("archive A");
        assert_eq!(
            project
                .sessions()
                .list_archived_for_checkout(runtime_a.checkout_id())
                .expect("archived checkout A sessions")
                .len(),
            1
        );
        project
            .sessions()
            .resolve_for_checkout(runtime_a.checkout_id(), &session_a)
            .expect("session resolves inside project checkout");
        project
            .sessions()
            .resolve_for_checkout(runtime_b.checkout_id(), &session_a)
            .expect("project session resolves in a sibling checkout");

        let checkout_a_id = runtime_a.checkout_id().clone();
        let runtime_a_weak = Arc::downgrade(&runtime_a);
        let store_weak = Arc::downgrade(&store);
        drop(runtime_a);
        drop(runtime_b);
        drop(registry);
        drop(store);
        let owned = project
            .checkout(&checkout_a_id)
            .expect("project strongly owns checkout runtime");
        assert!(Arc::ptr_eq(
            &owned,
            &runtime_a_weak.upgrade().expect("runtime remains owned")
        ));
        assert!(store_weak.upgrade().is_some());
        assert_eq!(
            project
                .sessions()
                .list()
                .expect("strong session catalog")
                .len(),
            1
        );
        assert_eq!(
            project
                .sessions()
                .list_archived()
                .expect("strong archived session catalog")
                .len(),
            1
        );
    }

    #[test]
    fn persisted_project_context_survives_runtime_retirement_and_reactivates_silently() {
        let temp = tempfile::tempdir().expect("temp root");
        let root = workspace_dir(temp.path(), "persisted-checkout");
        let identity = ProjectIdResolver::resolve(&root).expect("workspace identity");
        let store_dir = tempfile::tempdir().expect("session store");
        let store = Arc::new(
            crate::session::store::SessionStore::new(store_dir.path()).expect("session store"),
        );
        store
            .upsert_workspace_checkout(&crate::session::models::WorkspaceCheckoutRecord {
                checkout_id: identity.checkout_id.to_string(),
                project_id: identity.project_id.to_string(),
                root_path: root.display().to_string(),
                normalized_root: identity.normalized_root.clone(),
                last_opened_at: 1,
            })
            .expect("persist checkout");
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 1);

        registry.attach_session_store(&store).expect("attach store");

        let project = registry
            .project(&identity.project_id)
            .expect("durable project context");
        assert!(project.runtimes().is_empty());
        let dormant_sources = project
            .checkout_sources()
            .expect("dormant checkout sources");
        assert_eq!(dormant_sources.len(), 1);
        assert_eq!(dormant_sources[0].workspace_generation, None);

        let first = registry
            .activate_persisted_checkout(&identity.checkout_id)
            .expect("activate persisted checkout");
        let first_generation = first.generation();
        assert!(registry.remove_runtime_if_unleased(&first));
        assert!(project.runtimes().is_empty());
        assert_eq!(
            project
                .checkout_sources()
                .expect("retired checkout sources")[0]
                .workspace_generation,
            None
        );

        let restarted = registry
            .activate_persisted_checkout(&identity.checkout_id)
            .expect("silently reactivate checkout");
        assert!(restarted.generation() > first_generation);
        assert_eq!(
            project.checkout_sources().expect("active checkout sources")[0].workspace_generation,
            Some(restarted.generation())
        );
    }

    #[test]
    fn retirement_removes_the_project_owner_and_weak_checkout_index_together() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 1);
        let runtime = registry
            .register(workspace_dir(temp.path(), "owned-checkout"))
            .expect("register runtime");
        let project = registry
            .project(runtime.project_id())
            .expect("project context");
        let checkout_id = runtime.checkout_id().clone();
        let runtime_weak = Arc::downgrade(&runtime);

        assert!(project.checkout(&checkout_id).is_some());
        assert!(registry.remove_runtime_if_unleased(&runtime));
        assert!(registry.runtime(&checkout_id).is_none());
        assert!(project.checkout(&checkout_id).is_none());
        drop(runtime);
        assert!(runtime_weak.upgrade().is_none());
    }

    fn workspace_dir(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).expect("workspace directory");
        path
    }

    #[test]
    fn workspace_activity_priority_distinguishes_all_retention_levels() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 1);
        let runtime = registry
            .register(workspace_dir(temp.path(), "checkout"))
            .expect("register workspace");

        assert_eq!(
            runtime.activity_snapshot(std::time::Duration::MAX).priority,
            WorkspaceActivityPriority::BackgroundOpen
        );
        assert_eq!(
            runtime
                .activity_snapshot(std::time::Duration::ZERO)
                .priority,
            WorkspaceActivityPriority::Idle
        );

        let visible = runtime.acquire_lease(WorkspaceLeaseKind::VisiblePane);
        assert_eq!(
            runtime
                .activity_snapshot(std::time::Duration::ZERO)
                .priority,
            WorkspaceActivityPriority::VisiblePane
        );
        let running = runtime.acquire_lease(WorkspaceLeaseKind::RunningTask);
        let activity = runtime.activity_snapshot(std::time::Duration::ZERO);
        assert_eq!(activity.priority, WorkspaceActivityPriority::RunningTask);
        assert_eq!(activity.running_task_leases, 1);
        assert_eq!(activity.visible_pane_leases, 1);

        drop(running);
        assert_eq!(
            runtime
                .activity_snapshot(std::time::Duration::ZERO)
                .priority,
            WorkspaceActivityPriority::VisiblePane
        );
        drop(visible);
        assert_eq!(
            runtime
                .activity_snapshot(std::time::Duration::ZERO)
                .priority,
            WorkspaceActivityPriority::Idle
        );
    }

    #[test]
    fn checkout_runtimes_own_distinct_asset_operation_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 2);
        let runtime_a = registry
            .open_workspace(workspace_dir(temp.path(), "checkout-a"))
            .expect("runtime a");
        let runtime_b = registry
            .open_workspace(workspace_dir(temp.path(), "checkout-b"))
            .expect("runtime b");
        let core_a = runtime_a.core();
        let core_b = runtime_b.core();

        assert!(!Arc::ptr_eq(&core_a.asset_db(), &core_b.asset_db()));
        assert!(!Arc::ptr_eq(
            core_a.asset_preview_cache(),
            core_b.asset_preview_cache()
        ));
        assert!(!std::ptr::eq(
            core_a.asset_last_scan_info(),
            core_b.asset_last_scan_info()
        ));
        assert!(!std::ptr::eq(
            core_a.asset_scan_phase(),
            core_b.asset_scan_phase()
        ));
        assert!(!std::ptr::eq(
            core_a.dir_entries_page_cache(),
            core_b.dir_entries_page_cache()
        ));
        assert!(!std::ptr::eq(
            core_a.ref_graph_scan_tasks(),
            core_b.ref_graph_scan_tasks()
        ));
        assert!(!std::ptr::eq(
            core_a.asset_reconcile_tasks(),
            core_b.asset_reconcile_tasks()
        ));

        core_a
            .asset_scan_phase()
            .set(Some(crate::asset_db::types::ScanPhase::DirScan));
        assert!(core_b.asset_scan_phase().snapshot().is_none());
        core_a
            .asset_last_scan_info()
            .set(crate::commands::asset::LastScanInfo {
                finished_at_unix_ms: 1,
                duration_ms: 2,
                stats: crate::asset_db::types::ScanStats::default(),
            });
        assert!(core_b.asset_last_scan_info().snapshot().is_none());
    }

    #[test]
    fn concurrent_registration_is_single_flight_and_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::immediate();
        let registry = registry(temp.path(), Arc::clone(&factory), 2);
        let root = workspace_dir(temp.path(), "checkout");
        let barrier = Arc::new(std::sync::Barrier::new(12));

        let runtimes = std::thread::scope(|scope| {
            let handles = (0..12)
                .map(|_| {
                    let registry = Arc::clone(&registry);
                    let root = root.clone();
                    let barrier = Arc::clone(&barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        registry.open_workspace(root).expect("open workspace")
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("registration thread"))
                .collect::<Vec<_>>()
        });

        let first = &runtimes[0];
        assert!(runtimes.iter().all(|runtime| Arc::ptr_eq(first, runtime)));
        assert_eq!(factory.detects.load(Ordering::SeqCst), 1);
        assert_eq!(registry.checkout_count(), 1);
    }

    #[test]
    fn registration_initializer_completes_before_publication_and_reactivates_fast_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 2);
        let weak_registry = Arc::downgrade(&registry);
        let calls = Arc::new(AtomicUsize::new(0));
        let hook_calls = Arc::clone(&calls);
        registry
            .add_runtime_registration_hook(Arc::new(move |runtime| {
                let call = hook_calls.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    let registry = weak_registry.upgrade().expect("registry");
                    assert!(registry.runtime(runtime.checkout_id()).is_none());
                    assert!(registry.project(runtime.project_id()).is_none());
                }
                Ok(())
            }))
            .expect("registration initializer");

        let root = workspace_dir(temp.path(), "initialized-checkout");
        let first = registry.register(&root).expect("first registration");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let second = registry.register(&root).expect("fast-path registration");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn concurrent_runtime_observers_always_see_the_complete_project_hierarchy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 2);
        let root = workspace_dir(temp.path(), "hierarchy-checkout");
        let identity = ProjectIdResolver::resolve(&root).expect("workspace identity");
        let barrier = Arc::new(std::sync::Barrier::new(33));

        std::thread::scope(|scope| {
            let register_registry = Arc::clone(&registry);
            let register_root = root.clone();
            let register_barrier = Arc::clone(&barrier);
            let registration = scope.spawn(move || {
                register_barrier.wait();
                register_registry
                    .register(register_root)
                    .expect("register runtime")
            });

            let observers = (0..32)
                .map(|_| {
                    let registry = Arc::clone(&registry);
                    let barrier = Arc::clone(&barrier);
                    let checkout_id = identity.checkout_id.clone();
                    let project_id = identity.project_id.clone();
                    scope.spawn(move || {
                        barrier.wait();
                        let runtime = loop {
                            if let Some(runtime) = registry.runtime(&checkout_id) {
                                break runtime;
                            }
                            std::thread::yield_now();
                        };
                        assert_eq!(runtime.project_id(), &project_id);
                        let project = registry
                            .project(&project_id)
                            .expect("runtime project context must already be visible");
                        assert!(
                            project.checkout_ids().contains(&checkout_id),
                            "runtime checkout relation must precede global visibility"
                        );
                    })
                })
                .collect::<Vec<_>>();

            let runtime = registration.join().expect("registration thread");
            assert_eq!(runtime.checkout_id(), &identity.checkout_id);
            for observer in observers {
                observer.join().expect("observer thread");
            }
        });
    }

    #[test]
    fn retirement_joins_local_reference_workers_before_reopening_checkout() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 2);
        let root = workspace_dir(temp.path(), "retired-checkout");
        let first = registry.register(&root).expect("first runtime");
        let first_generation = first.generation();
        let watcher_state = first.core().knowledge_operations().local_reference_watcher;
        let (observed_stop, worker_exited) =
            watcher_state.install_test_worker("live-reference", Duration::from_millis(80));

        let retire_registry = Arc::clone(&registry);
        let retire_runtime = Arc::clone(&first);
        let retirement = std::thread::spawn(move || {
            assert!(retire_registry.remove_runtime_if_unleased(&retire_runtime));
        });
        let wait_started = Instant::now();
        while !observed_stop.load(Ordering::Acquire) {
            assert!(
                wait_started.elapsed() < Duration::from_secs(2),
                "retirement did not stop the live worker"
            );
            std::thread::yield_now();
        }

        let reopen_registry = Arc::clone(&registry);
        let reopen_root = root.clone();
        let (reopened_tx, reopened_rx) = std::sync::mpsc::channel();
        let reopen = std::thread::spawn(move || {
            let runtime = reopen_registry
                .register(reopen_root)
                .expect("reopened runtime");
            reopened_tx
                .send((runtime, worker_exited.load(Ordering::Acquire)))
                .expect("reopen result");
        });

        assert!(
            reopened_rx.recv_timeout(Duration::from_millis(20)).is_err(),
            "reopen crossed the registration gate before worker join"
        );
        let (second, worker_had_exited) = reopened_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reopen after retirement");
        assert!(worker_had_exited);
        assert!(second.generation() > first_generation);
        assert_eq!(watcher_state.live_watcher_count(), 0);

        retirement.join().expect("retirement thread");
        reopen.join().expect("reopen thread");
    }

    #[test]
    fn knowledge_operation_state_is_stable_per_checkout_and_isolated_between_checkouts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::immediate();
        let registry = registry(temp.path(), factory, 2);
        let runtime_a = registry
            .register(workspace_dir(temp.path(), "checkout-a"))
            .expect("register checkout a");
        let runtime_b = registry
            .register(workspace_dir(temp.path(), "checkout-b"))
            .expect("register checkout b");

        let first_a = runtime_a.core().knowledge_operations();
        let second_a = runtime_a.core().knowledge_operations();
        let state_b = runtime_b.core().knowledge_operations();

        assert!(Arc::ptr_eq(
            &first_a.unity_reference_import.0,
            &second_a.unity_reference_import.0
        ));
        assert!(!Arc::ptr_eq(
            &first_a.unity_reference_import.0,
            &state_b.unity_reference_import.0
        ));
        assert!(!Arc::ptr_eq(
            &first_a.local_reference_import.0,
            &state_b.local_reference_import.0
        ));
        assert!(!Arc::ptr_eq(
            &first_a.feishu_reference_import.0,
            &state_b.feishu_reference_import.0
        ));
    }

    #[test]
    fn workspace_ref_resolution_holds_a_lease_and_rejects_stale_generation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::immediate();
        let registry = registry(temp.path(), factory, 1);
        let runtime = registry
            .register(workspace_dir(temp.path(), "checkout"))
            .expect("register workspace");
        let reference = WorkspaceRef::for_runtime(&runtime);

        let resolved = registry
            .resolve_workspace_ref(&reference)
            .expect("resolve current workspace ref");
        assert!(Arc::ptr_eq(resolved.runtime(), &runtime));
        assert_eq!(runtime.lease_count(), 1);
        drop(resolved);
        assert_eq!(runtime.lease_count(), 0);

        let stale = WorkspaceRef::new(
            runtime.checkout_id().clone(),
            Some(runtime.generation().saturating_add(1)),
        );
        assert!(matches!(
            registry.resolve_workspace_ref(&stale),
            Err(WorkspaceResolveError::StaleGeneration {
                expected_generation,
                actual_generation,
                ..
            }) if expected_generation == runtime.generation().saturating_add(1)
                && actual_generation == runtime.generation()
        ));
        assert_eq!(runtime.lease_count(), 0);
    }

    #[tokio::test]
    async fn ordinary_directory_runtime_keeps_core_agent_capabilities_without_services() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = Arc::new(AppConfig::load_from_path(
            &temp.path().join("plain-config.json"),
        ));
        let policy = Arc::new(ResourcePolicyStore::from_config(config).expect("policy"));
        let registry = ProjectRegistry::new(policy, Vec::new());
        let root = workspace_dir(temp.path(), "plain-checkout");
        let runtime = registry.register(&root).expect("plain runtime");
        assert!(runtime.services().detected_kinds().is_empty());
        let execution = registry
            .execution_context(runtime.checkout_id(), &[])
            .await
            .expect("core execution context");
        assert_eq!(execution.root(), root.as_path());
        assert!(execution.service_bindings.is_empty());
    }

    #[tokio::test]
    async fn activation_policy_can_be_restored_before_lazy_start() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::immediate();
        let registry = registry(temp.path(), factory, 1);
        let runtime = registry
            .register(workspace_dir(temp.path(), "checkout"))
            .expect("register workspace");
        assert_eq!(
            runtime.services().activation_policy(ServiceKind::Unity),
            Some(ServiceActivationPolicy::Lazy)
        );
        runtime
            .services()
            .set_activation_policy(ServiceKind::Unity, ServiceActivationPolicy::Disabled)
            .expect("disable service");
        let error = match registry
            .execution_context(runtime.checkout_id(), &[ServiceKind::Unity])
            .await
        {
            Ok(_) => panic!("disabled service started"),
            Err(error) => error,
        };
        assert!(error.contains("disabled"), "unexpected error: {error}");
        runtime
            .services()
            .set_activation_policy(ServiceKind::Unity, ServiceActivationPolicy::Manual)
            .expect("restore service policy");
        assert_eq!(
            runtime.services().activation_policy(ServiceKind::Unity),
            Some(ServiceActivationPolicy::Manual)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_execution_contexts_merge_one_service_start() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::blocked();
        let registry = registry(temp.path(), Arc::clone(&factory), 1);
        let runtime = registry
            .register(workspace_dir(temp.path(), "checkout"))
            .expect("register workspace");
        let checkout_id = runtime.checkout_id().clone();

        let first_registry = Arc::clone(&registry);
        let first_checkout = checkout_id.clone();
        let first = tokio::spawn(async move {
            first_registry
                .execution_context(&first_checkout, &[ServiceKind::Unity])
                .await
        });
        let second_registry = Arc::clone(&registry);
        let second_checkout = checkout_id.clone();
        let second = tokio::spawn(async move {
            second_registry
                .execution_context(&second_checkout, &[ServiceKind::Unity])
                .await
        });

        factory.control.wait_until_entered().await;
        factory.control.release_one();
        let first = first.await.expect("first join").expect("first context");
        let second = second.await.expect("second join").expect("second context");

        assert_eq!(factory.creates.load(Ordering::SeqCst), 1);
        assert_eq!(factory.starts.load(Ordering::SeqCst), 1);
        assert_eq!(
            first
                .binding(ServiceKind::Unity)
                .expect("first binding")
                .runtime_generation,
            second
                .binding(ServiceKind::Unity)
                .expect("second binding")
                .runtime_generation
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn capacity_reservation_is_atomic_across_checkouts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::blocked();
        let registry = registry(temp.path(), Arc::clone(&factory), 1);
        let first_runtime = registry
            .register(workspace_dir(temp.path(), "first"))
            .expect("register first");
        let second_runtime = registry
            .register(workspace_dir(temp.path(), "second"))
            .expect("register second");

        let first_registry = Arc::clone(&registry);
        let first_checkout = first_runtime.checkout_id().clone();
        let first = tokio::spawn(async move {
            first_registry
                .execution_context(&first_checkout, &[ServiceKind::Unity])
                .await
        });
        factory.control.wait_until_entered().await;

        let error = match registry
            .execution_context(second_runtime.checkout_id(), &[ServiceKind::Unity])
            .await
        {
            Ok(_) => panic!("second checkout exceeded reserved capacity"),
            Err(error) => error,
        };
        assert!(
            error.contains("capacity is busy"),
            "unexpected error: {error}"
        );
        assert_eq!(factory.starts.load(Ordering::SeqCst), 1);

        factory.control.release_one();
        first.await.expect("first join").expect("first context");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runtime_rebuild_and_service_restart_advance_global_generations() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::immediate();
        let registry = registry(temp.path(), factory, 1);
        let root = workspace_dir(temp.path(), "checkout");
        let first_runtime = registry.register(&root).expect("register workspace");
        let first_runtime_generation = first_runtime.generation();
        let first_context = registry
            .execution_context(first_runtime.checkout_id(), &[ServiceKind::Unity])
            .await
            .expect("first context");
        let stale_binding = first_context
            .binding(ServiceKind::Unity)
            .expect("first binding")
            .clone();
        let resolved = stale_binding.resolve().expect("resolve first binding");
        let first_identity = resolved.service.identity();
        drop(resolved);
        drop(first_context);

        registry
            .stop_service(&first_runtime, ServiceKind::Unity)
            .await
            .expect("stop first service");
        assert!(matches!(
            stale_binding.resolve(),
            Err(ServiceBindingError::Stale { .. })
        ));
        assert!(!first_runtime
            .services()
            .is_current_identity(&first_identity));
        assert!(registry.remove_runtime_if_unleased(&first_runtime));

        let second_runtime = registry.register(&root).expect("re-register workspace");
        assert!(second_runtime.generation() > first_runtime_generation);
        let second_context = registry
            .execution_context(second_runtime.checkout_id(), &[ServiceKind::Unity])
            .await
            .expect("second context");
        let resolved = second_context
            .resolve_service(ServiceKind::Unity)
            .expect("resolve second binding");
        let second_identity = resolved.service.identity();
        assert!(second_identity.runtime_generation > first_identity.runtime_generation);
        assert_eq!(
            second_identity.service_instance_id,
            first_identity.service_instance_id
        );
        assert!(second_runtime
            .services()
            .is_current_identity(&second_identity));
        assert!(!second_runtime
            .services()
            .is_current_identity(&first_identity));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn execution_acquires_workspace_lease_before_start_await_and_explicit_retirement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::blocked();
        let registry = registry(temp.path(), Arc::clone(&factory), 2);
        let active_runtime = registry
            .register(workspace_dir(temp.path(), "active"))
            .expect("register active");
        let background_runtime = registry
            .register(workspace_dir(temp.path(), "background"))
            .expect("register background");

        let active_checkout = active_runtime.checkout_id().clone();
        let execution_registry = Arc::clone(&registry);
        let execution = tokio::spawn(async move {
            execution_registry
                .execution_context(&active_checkout, &[ServiceKind::Unity])
                .await
        });
        factory.control.wait_until_entered().await;
        assert_eq!(active_runtime.lease_count(), 1);

        assert!(!registry.remove_runtime_if_unleased(&active_runtime));
        assert!(registry.remove_runtime_if_unleased(&background_runtime));
        assert!(registry.runtime(active_runtime.checkout_id()).is_some());
        assert!(registry.runtime(background_runtime.checkout_id()).is_none());

        factory.control.release_one();
        let context = execution.await.expect("execution join").expect("context");
        assert!(!registry.remove_runtime_if_unleased(&active_runtime));
        assert!(registry.runtime(active_runtime.checkout_id()).is_some());

        drop(context);
        registry
            .stop_service(&active_runtime, ServiceKind::Unity)
            .await
            .expect("stop active service");
        assert!(registry.remove_runtime_if_unleased(&active_runtime));
        assert!(registry.runtime(active_runtime.checkout_id()).is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn policy_shrink_keeps_service_with_active_tool_lease() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::immediate();
        let registry = registry(temp.path(), factory, 2);
        let first_runtime = registry
            .register(workspace_dir(temp.path(), "first"))
            .expect("register first");
        let second_runtime = registry
            .register(workspace_dir(temp.path(), "second"))
            .expect("register second");
        let first_context = registry
            .execution_context(first_runtime.checkout_id(), &[ServiceKind::Unity])
            .await
            .expect("first context");
        let second_context = registry
            .execution_context(second_runtime.checkout_id(), &[ServiceKind::Unity])
            .await
            .expect("second context");
        let active_tool_lease = first_context
            .resolve_service(ServiceKind::Unity)
            .expect("active tool lease");
        drop(second_context);

        let mut limits = registry.resource_policy().snapshot().limits;
        limits.max_running_workspace_services = 1;
        registry
            .resource_policy()
            .update(limits)
            .expect("shrink policy");
        registry.converge_resource_policy().await;

        assert_eq!(active_tool_lease.service.status(), ServiceStatus::Running);
        assert!(
            first_runtime
                .services()
                .is_running(ServiceKind::Unity)
                .await
        );
        assert!(
            !second_runtime
                .services()
                .is_running(ServiceKind::Unity)
                .await
        );
        drop(active_tool_lease);
        drop(first_context);
    }

    #[tokio::test]
    async fn idle_unity_service_reap_stops_service_and_keeps_workspace_runtime() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 1);
        let runtime = registry
            .register(workspace_dir(temp.path(), "unity-checkout"))
            .expect("register workspace");
        let context = registry
            .execution_context(runtime.checkout_id(), &[ServiceKind::Unity])
            .await
            .expect("start Unity service");
        let service_lease = context
            .resolve_service(ServiceKind::Unity)
            .expect("resolve Unity service");
        let service_tracker = service_lease.service.lease_tracker();
        drop(service_lease);
        drop(context);
        service_tracker.set_idle_for_test(Duration::from_secs(3601));

        registry.reap_idle_resources().await;

        assert!(!runtime.services().is_running(ServiceKind::Unity).await);
        assert!(registry.runtime(runtime.checkout_id()).is_some());
    }

    #[tokio::test]
    async fn policy_shrink_preserves_visible_pane_service_before_background_service() {
        let temp = tempfile::tempdir().expect("tempdir");
        let factory = FakeServiceFactory::immediate();
        let registry = registry(temp.path(), factory, 2);
        let visible_runtime = registry
            .register(workspace_dir(temp.path(), "visible"))
            .expect("register visible");
        let background_runtime = registry
            .register(workspace_dir(temp.path(), "background"))
            .expect("register background");
        let visible_context = registry
            .execution_context(visible_runtime.checkout_id(), &[ServiceKind::Unity])
            .await
            .expect("start visible service");
        let background_context = registry
            .execution_context(background_runtime.checkout_id(), &[ServiceKind::Unity])
            .await
            .expect("start background service");
        drop(visible_context);
        drop(background_context);

        let panes = WindowContextRegistry::new();
        panes
            .focus("main", "left", Arc::clone(&visible_runtime), 1)
            .expect("focus visible checkout");
        let mut limits = registry.resource_policy().snapshot().limits;
        limits.max_running_workspace_services = 1;
        registry
            .resource_policy()
            .update(limits)
            .expect("shrink service policy");

        registry.converge_resource_policy().await;

        assert!(
            visible_runtime
                .services()
                .is_running(ServiceKind::Unity)
                .await
        );
        assert!(
            !background_runtime
                .services()
                .is_running(ServiceKind::Unity)
                .await
        );
    }

    #[tokio::test]
    async fn idle_resource_reap_preserves_visible_and_background_workspace_runtimes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = registry(temp.path(), FakeServiceFactory::immediate(), 1);
        let visible_runtime = registry
            .register(workspace_dir(temp.path(), "visible"))
            .expect("register visible");
        let background_runtime = registry
            .register(workspace_dir(temp.path(), "background"))
            .expect("register background");
        let panes = WindowContextRegistry::new();
        panes
            .focus("main", "left", Arc::clone(&visible_runtime), 1)
            .expect("focus visible checkout");
        for runtime in [&visible_runtime, &background_runtime] {
            *runtime
                .leases
                .last_used_at
                .lock()
                .expect("workspace last-used lock") = Instant::now() - Duration::from_secs(601);
        }

        registry.reap_idle_resources().await;

        assert!(registry.runtime(visible_runtime.checkout_id()).is_some());
        assert!(registry.runtime(background_runtime.checkout_id()).is_some());
    }
}
