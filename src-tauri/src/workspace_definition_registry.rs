use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use crate::agent::definition::AgentDefRegistry;
use crate::workspace_service::identity::ResolvedWorkspaceIdentity;
use crate::workspace_service::{CheckoutId, WorkspaceRuntime};

type AgentRegistryLoader =
    dyn Fn(Option<PathBuf>, PathBuf) -> AgentDefRegistry + Send + Sync + 'static;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DefinitionCacheKey {
    checkout_id: CheckoutId,
    workspace_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DefinitionEpoch {
    app_base: u64,
    checkout: u64,
}

struct CachedDefinition {
    epoch: DefinitionEpoch,
    registry: Arc<AgentDefRegistry>,
}

pub(crate) struct DefinitionCacheEntry {
    root: PathBuf,
    normalized_root: String,
    cached: RwLock<Option<CachedDefinition>>,
    removed: AtomicBool,
}

impl DefinitionCacheEntry {
    pub(crate) fn new(identity: &ResolvedWorkspaceIdentity) -> Self {
        Self {
            root: identity.root.clone(),
            normalized_root: identity.normalized_root.clone(),
            cached: RwLock::new(None),
            removed: AtomicBool::new(false),
        }
    }

    fn cached_for(&self, epoch: DefinitionEpoch) -> Result<Option<Arc<AgentDefRegistry>>, String> {
        let cached = self
            .cached
            .read()
            .map_err(|error| format!("workspace definition cache read lock poisoned: {error}"))?;
        Ok(cached
            .as_ref()
            .filter(|cached| cached.epoch == epoch)
            .map(|cached| Arc::clone(&cached.registry)))
    }
}

#[derive(Default)]
struct DefinitionRegistryState {
    app_base_epoch: u64,
    checkout_epochs: HashMap<CheckoutId, u64>,
    entries: HashMap<DefinitionCacheKey, Arc<DefinitionCacheEntry>>,
    checkout_build_gates: HashMap<CheckoutId, Arc<tokio::sync::Mutex<()>>>,
}

/// Coordinates the app Agent base and indexes immutable snapshots physically
/// owned by each live `WorkspaceRuntime`.
///
/// Cache identity includes the runtime generation. Invalidations advance logical
/// epochs, allowing an in-flight disk scan to finish without ever committing a
/// stale snapshot. A checkout-level build gate also prevents concurrent scans
/// for different generations of the same checkout.
pub struct WorkspaceDefinitionRegistry {
    app_agent_dir: Option<PathBuf>,
    loader: Arc<AgentRegistryLoader>,
    state: Mutex<DefinitionRegistryState>,
}

impl WorkspaceDefinitionRegistry {
    pub fn new(app_agent_dir: Option<PathBuf>) -> Self {
        Self::with_loader(app_agent_dir, Arc::new(load_agent_registry))
    }

    fn with_loader(app_agent_dir: Option<PathBuf>, loader: Arc<AgentRegistryLoader>) -> Self {
        Self {
            app_agent_dir,
            loader,
            state: Mutex::new(DefinitionRegistryState::default()),
        }
    }

    /// Returns the current immutable Agent registry for `runtime`.
    ///
    /// Concurrent misses for the same checkout share one build. If either the
    /// app base or checkout changes during the build, the stale result is
    /// discarded and rebuilt while the checkout build gate remains held.
    pub async fn snapshot(
        &self,
        runtime: &WorkspaceRuntime,
    ) -> Result<Arc<AgentDefRegistry>, String> {
        let (key, entry, build_gate, initial_epoch) = self.entry_for_runtime(runtime)?;

        if !entry.removed.load(Ordering::Acquire)
            && runtime.generation() == key.workspace_generation
        {
            if let Some(cached) = entry.cached_for(initial_epoch)? {
                return Ok(cached);
            }
        }

        let _build_guard = build_gate.lock().await;
        loop {
            if runtime.generation() != key.workspace_generation {
                return Err(format!(
                    "workspace runtime generation changed while resolving Agent definitions for checkout {} (expected {}, found {})",
                    key.checkout_id,
                    key.workspace_generation,
                    runtime.generation()
                ));
            }

            let build_epoch = {
                let state = self.lock_state()?;
                self.ensure_entry_is_current(&state, &key, &entry)?;
                target_epoch(&state, &key.checkout_id)
            };

            if let Some(cached) = entry.cached_for(build_epoch)? {
                return Ok(cached);
            }

            let loader = Arc::clone(&self.loader);
            let app_agent_dir = self.app_agent_dir.clone();
            let workspace_root = entry.root.clone();
            let built =
                tokio::task::spawn_blocking(move || (loader)(app_agent_dir, workspace_root))
                    .await
                    .map_err(|error| {
                        format!(
                            "failed to build Agent definitions for checkout {}: {error}",
                            key.checkout_id
                        )
                    })?;
            let built = Arc::new(built);

            if runtime.generation() != key.workspace_generation {
                return Err(format!(
                    "workspace runtime generation changed while building Agent definitions for checkout {} (expected {}, found {})",
                    key.checkout_id,
                    key.workspace_generation,
                    runtime.generation()
                ));
            }

            let state = self.lock_state()?;
            self.ensure_entry_is_current(&state, &key, &entry)?;
            if target_epoch(&state, &key.checkout_id) != build_epoch {
                drop(state);
                continue;
            }

            let mut cached = entry.cached.write().map_err(|error| {
                format!("workspace definition cache write lock poisoned: {error}")
            })?;
            *cached = Some(CachedDefinition {
                epoch: build_epoch,
                registry: Arc::clone(&built),
            });
            return Ok(built);
        }
    }

    /// Invalidates only the checkout overlay and returns its new epoch.
    pub fn invalidate_checkout(&self, checkout_id: &CheckoutId) -> Result<u64, String> {
        let mut state = self.lock_state()?;
        let epoch = state
            .checkout_epochs
            .entry(checkout_id.clone())
            .or_default();
        advance_epoch(epoch, "checkout Agent definition")
    }

    /// Invalidates the shared app Agent base for all checkouts and returns its
    /// new epoch. Existing snapshots rebuild lazily on their next access.
    pub fn invalidate_app_base(&self) -> Result<u64, String> {
        let mut state = self.lock_state()?;
        advance_epoch(&mut state.app_base_epoch, "app Agent definition")
    }

    /// Removes one retired runtime generation without disturbing a newer
    /// generation for the same checkout. In-flight builds observe `removed`
    /// before commit and return an error.
    pub fn remove_generation(
        &self,
        checkout_id: &CheckoutId,
        workspace_generation: u64,
    ) -> Result<bool, String> {
        let key = DefinitionCacheKey {
            checkout_id: checkout_id.clone(),
            workspace_generation,
        };
        let mut state = self.lock_state()?;
        let Some(entry) = state.entries.remove(&key) else {
            return Ok(false);
        };
        entry.removed.store(true, Ordering::Release);
        Ok(true)
    }

    fn entry_for_runtime(
        &self,
        runtime: &WorkspaceRuntime,
    ) -> Result<
        (
            DefinitionCacheKey,
            Arc<DefinitionCacheEntry>,
            Arc<tokio::sync::Mutex<()>>,
            DefinitionEpoch,
        ),
        String,
    > {
        let key = DefinitionCacheKey {
            checkout_id: runtime.checkout_id().clone(),
            workspace_generation: runtime.generation(),
        };
        let mut state = self.lock_state()?;
        let runtime_entry = runtime.definition_cache_entry();
        let entry = state
            .entries
            .entry(key.clone())
            .or_insert_with(|| Arc::clone(&runtime_entry))
            .clone();
        if !Arc::ptr_eq(&entry, &runtime_entry) {
            return Err(format!(
                "workspace definition cache generation collision for checkout {} generation {}",
                key.checkout_id, key.workspace_generation
            ));
        }
        if entry.normalized_root != runtime.normalized_root() {
            return Err(format!(
                "workspace definition cache identity mismatch for checkout {} generation {}",
                key.checkout_id, key.workspace_generation
            ));
        }
        let build_gate = state
            .checkout_build_gates
            .entry(key.checkout_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let epoch = target_epoch(&state, &key.checkout_id);
        Ok((key, entry, build_gate, epoch))
    }

    fn ensure_entry_is_current(
        &self,
        state: &DefinitionRegistryState,
        key: &DefinitionCacheKey,
        entry: &Arc<DefinitionCacheEntry>,
    ) -> Result<(), String> {
        let entry_matches = state
            .entries
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, entry));
        if entry.removed.load(Ordering::Acquire) || !entry_matches {
            return Err(format!(
                "workspace Agent definition generation was removed for checkout {} generation {}",
                key.checkout_id, key.workspace_generation
            ));
        }
        Ok(())
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, DefinitionRegistryState>, String> {
        self.state
            .lock()
            .map_err(|error| format!("workspace definition registry lock poisoned: {error}"))
    }
}

fn target_epoch(state: &DefinitionRegistryState, checkout_id: &CheckoutId) -> DefinitionEpoch {
    DefinitionEpoch {
        app_base: state.app_base_epoch,
        checkout: state
            .checkout_epochs
            .get(checkout_id)
            .copied()
            .unwrap_or_default(),
    }
}

fn advance_epoch(epoch: &mut u64, label: &str) -> Result<u64, String> {
    *epoch = epoch
        .checked_add(1)
        .ok_or_else(|| format!("{label} epoch space exhausted"))?;
    Ok(*epoch)
}

fn load_agent_registry(
    app_agent_dir: Option<PathBuf>,
    workspace_root: PathBuf,
) -> AgentDefRegistry {
    let working_dir = workspace_root.to_string_lossy().into_owned();
    let project_agent_dir = workspace_root.join("Locus").join("agent");
    let project_agent_dir = project_agent_dir.is_dir().then_some(project_agent_dir);
    let plugin_sources = crate::plugin::installed_agent_sources(&working_dir);
    AgentDefRegistry::load_with_plugins(
        app_agent_dir.as_deref(),
        project_agent_dir.as_deref(),
        &plugin_sources,
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use crate::agent::definition::AgentDefRegistry;
    use crate::workspace_service::identity::ProjectIdResolver;
    use crate::workspace_service::WorkspaceRuntime;

    use super::{load_agent_registry, AgentRegistryLoader, WorkspaceDefinitionRegistry};

    fn write_workspace_agent(root: &Path, prompt: &str) {
        let agent_dir = root.join("Locus").join("agent").join("dev");
        fs::create_dir_all(&agent_dir).expect("workspace Agent directory");
        fs::write(
            agent_dir.join("config.json"),
            r#"{"name":"Unity Dev","description":"Workspace Agent","tools":[],"default":true}"#,
        )
        .expect("workspace Agent config");
        fs::write(agent_dir.join("system.md"), prompt).expect("workspace Agent prompt");
    }

    fn runtime(root: &Path, generation: u64) -> Arc<WorkspaceRuntime> {
        let identity = ProjectIdResolver::resolve(root).expect("workspace identity");
        WorkspaceRuntime::new(identity, Vec::new(), generation)
    }

    fn prompt(registry: &AgentDefRegistry) -> &str {
        &registry
            .get("dev")
            .expect("workspace dev Agent")
            .system_prompt
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn snapshots_are_isolated_by_checkout_and_generation() {
        let temp = tempfile::tempdir().expect("temp root");
        let root_a = temp.path().join("checkout-a");
        let root_b = temp.path().join("checkout-b");
        write_workspace_agent(&root_a, "prompt-a");
        write_workspace_agent(&root_b, "prompt-b");

        let runtime_a1 = runtime(&root_a, 1);
        let runtime_a2 = runtime(&root_a, 2);
        let runtime_b1 = runtime(&root_b, 1);
        let definitions = WorkspaceDefinitionRegistry::new(None);

        let a1 = definitions
            .snapshot(runtime_a1.as_ref())
            .await
            .expect("checkout A generation 1 snapshot");
        let a1_again = definitions
            .snapshot(runtime_a1.as_ref())
            .await
            .expect("cached checkout A generation 1 snapshot");
        let a2 = definitions
            .snapshot(runtime_a2.as_ref())
            .await
            .expect("checkout A generation 2 snapshot");
        let b1 = definitions
            .snapshot(runtime_b1.as_ref())
            .await
            .expect("checkout B snapshot");

        assert_eq!(prompt(&a1), "prompt-a");
        assert_eq!(prompt(&b1), "prompt-b");
        assert!(Arc::ptr_eq(&a1, &a1_again));
        assert!(!Arc::ptr_eq(&a1, &a2));

        assert!(definitions
            .remove_generation(runtime_a1.checkout_id(), 1)
            .expect("remove generation 1"));
        let retired_a1 = match definitions.snapshot(runtime_a1.as_ref()).await {
            Ok(_) => panic!("retired runtime generation must stay unavailable"),
            Err(error) => error,
        };
        let cached_a2 = definitions
            .snapshot(runtime_a2.as_ref())
            .await
            .expect("generation 2 remains cached");
        assert!(retired_a1.contains("generation was removed"));
        assert!(Arc::ptr_eq(&a2, &cached_a2));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn checkout_and_app_invalidations_rebuild_the_expected_snapshots() {
        let temp = tempfile::tempdir().expect("temp root");
        let root_a = temp.path().join("checkout-a");
        let root_b = temp.path().join("checkout-b");
        write_workspace_agent(&root_a, "prompt-a-v1");
        write_workspace_agent(&root_b, "prompt-b");
        let runtime_a = runtime(&root_a, 1);
        let runtime_b = runtime(&root_b, 1);
        let definitions = WorkspaceDefinitionRegistry::new(None);

        let a_before = definitions.snapshot(runtime_a.as_ref()).await.unwrap();
        let b_before = definitions.snapshot(runtime_b.as_ref()).await.unwrap();

        write_workspace_agent(&root_a, "prompt-a-v2");
        definitions
            .invalidate_checkout(runtime_a.checkout_id())
            .expect("invalidate checkout A");
        let a_after_checkout = definitions.snapshot(runtime_a.as_ref()).await.unwrap();
        let b_after_checkout = definitions.snapshot(runtime_b.as_ref()).await.unwrap();
        assert_eq!(prompt(&a_after_checkout), "prompt-a-v2");
        assert!(!Arc::ptr_eq(&a_before, &a_after_checkout));
        assert!(Arc::ptr_eq(&b_before, &b_after_checkout));

        definitions
            .invalidate_app_base()
            .expect("invalidate app Agent base");
        let a_after_app = definitions.snapshot(runtime_a.as_ref()).await.unwrap();
        let b_after_app = definitions.snapshot(runtime_b.as_ref()).await.unwrap();
        assert!(!Arc::ptr_eq(&a_after_checkout, &a_after_app));
        assert!(!Arc::ptr_eq(&b_after_checkout, &b_after_app));
    }

    struct LoaderGate {
        calls: AtomicUsize,
        started: Mutex<bool>,
        started_changed: Condvar,
        released: Mutex<bool>,
        released_changed: Condvar,
    }

    impl LoaderGate {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                started: Mutex::new(false),
                started_changed: Condvar::new(),
                released: Mutex::new(false),
                released_changed: Condvar::new(),
            }
        }

        fn finish_first_load_when_released(&self) {
            if self.calls.fetch_add(1, Ordering::AcqRel) != 0 {
                return;
            }
            let mut started = self.started.lock().expect("loader started lock");
            *started = true;
            self.started_changed.notify_all();
            drop(started);

            let mut released = self.released.lock().expect("loader release lock");
            let deadline = Instant::now() + Duration::from_secs(10);
            while !*released {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .expect("timed out waiting to release Agent definition loader");
                let (next, timeout) = self
                    .released_changed
                    .wait_timeout(released, remaining)
                    .expect("loader release wait");
                released = next;
                assert!(
                    *released || !timeout.timed_out(),
                    "timed out waiting to release Agent definition loader"
                );
            }
        }

        fn wait_until_started(&self) {
            let mut started = self.started.lock().expect("loader started lock");
            let deadline = Instant::now() + Duration::from_secs(10);
            while !*started {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .expect("timed out waiting for Agent definition loader");
                let (next, timeout) = self
                    .started_changed
                    .wait_timeout(started, remaining)
                    .expect("loader started wait");
                started = next;
                assert!(
                    *started || !timeout.timed_out(),
                    "timed out waiting for Agent definition loader"
                );
            }
        }

        fn release(&self) {
            let mut released = self.released.lock().expect("loader release lock");
            *released = true;
            self.released_changed.notify_all();
        }
    }

    fn gated_loader(gate: Arc<LoaderGate>) -> Arc<AgentRegistryLoader> {
        Arc::new(move |app_agent_dir, workspace_root| {
            let registry = load_agent_registry(app_agent_dir, workspace_root);
            gate.finish_first_load_when_released();
            registry
        })
    }

    async fn wait_until_loader_started(gate: Arc<LoaderGate>) {
        tokio::task::spawn_blocking(move || gate.wait_until_started())
            .await
            .expect("loader start waiter");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_snapshots_for_one_checkout_share_one_build() {
        let temp = tempfile::tempdir().expect("temp root");
        write_workspace_agent(temp.path(), "singleflight");
        let runtime = runtime(temp.path(), 1);
        let gate = Arc::new(LoaderGate::new());
        let definitions = Arc::new(WorkspaceDefinitionRegistry::with_loader(
            None,
            gated_loader(Arc::clone(&gate)),
        ));

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let definitions = Arc::clone(&definitions);
            let runtime = Arc::clone(&runtime);
            tasks.push(tokio::spawn(async move {
                definitions.snapshot(runtime.as_ref()).await
            }));
        }

        wait_until_loader_started(Arc::clone(&gate)).await;
        gate.release();

        let mut snapshots = Vec::new();
        for task in tasks {
            snapshots.push(
                task.await
                    .expect("snapshot task")
                    .expect("Agent definition snapshot"),
            );
        }
        assert_eq!(gate.calls.load(Ordering::Acquire), 1);
        for snapshot in &snapshots[1..] {
            assert!(Arc::ptr_eq(&snapshots[0], snapshot));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn invalidation_during_build_discards_the_stale_result() {
        let temp = tempfile::tempdir().expect("temp root");
        write_workspace_agent(temp.path(), "prompt-v1");
        let runtime = runtime(temp.path(), 1);
        let gate = Arc::new(LoaderGate::new());
        let definitions = Arc::new(WorkspaceDefinitionRegistry::with_loader(
            None,
            gated_loader(Arc::clone(&gate)),
        ));

        let snapshot_task = {
            let definitions = Arc::clone(&definitions);
            let runtime = Arc::clone(&runtime);
            tokio::spawn(async move { definitions.snapshot(runtime.as_ref()).await })
        };
        wait_until_loader_started(Arc::clone(&gate)).await;

        write_workspace_agent(temp.path(), "prompt-v2");
        definitions
            .invalidate_checkout(runtime.checkout_id())
            .expect("invalidate during build");
        gate.release();

        let snapshot = snapshot_task
            .await
            .expect("snapshot task")
            .expect("rebuilt snapshot");
        assert_eq!(prompt(&snapshot), "prompt-v2");
        assert_eq!(gate.calls.load(Ordering::Acquire), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn removing_a_generation_prevents_an_inflight_commit() {
        let temp = tempfile::tempdir().expect("temp root");
        write_workspace_agent(temp.path(), "removed-generation");
        let runtime = runtime(temp.path(), 7);
        let gate = Arc::new(LoaderGate::new());
        let definitions = Arc::new(WorkspaceDefinitionRegistry::with_loader(
            None,
            gated_loader(Arc::clone(&gate)),
        ));

        let snapshot_task = {
            let definitions = Arc::clone(&definitions);
            let runtime = Arc::clone(&runtime);
            tokio::spawn(async move { definitions.snapshot(runtime.as_ref()).await })
        };
        wait_until_loader_started(Arc::clone(&gate)).await;

        assert!(definitions
            .remove_generation(runtime.checkout_id(), runtime.generation())
            .expect("remove in-flight generation"));
        gate.release();

        let error = match snapshot_task.await.expect("snapshot task") {
            Ok(_) => panic!("removed generation must reject stale commit"),
            Err(error) => error,
        };
        assert!(error.contains("generation was removed"));
        assert_eq!(gate.calls.load(Ordering::Acquire), 1);
    }
}
