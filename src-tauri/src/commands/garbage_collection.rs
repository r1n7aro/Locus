use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use tauri::State;

use crate::session::store::SessionStore;
use crate::sqlite_maint::GarbageCollectionResult;
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};

static RUNNING: AtomicBool = AtomicBool::new(false);

struct CollectionGuard;

impl Drop for CollectionGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseCollection {
    kind: &'static str,
    path: String,
    result: Option<GarbageCollectionResult>,
    error: Option<String>,
    skipped: bool,
}

impl DatabaseCollection {
    fn from_result(
        kind: &'static str,
        path: &Path,
        result: Result<GarbageCollectionResult, String>,
    ) -> Self {
        let (result, error) = match result {
            Ok(result) => (Some(result), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            kind,
            path: path.to_string_lossy().into_owned(),
            result,
            error,
            skipped: false,
        }
    }
}

/// Compact the global session store and only the explicitly selected checkout.
/// Never use database initializers here: they may rebuild outdated project caches.
#[tauri::command]
pub async fn garbage_collection(
    workspace_ref: Option<WorkspaceRef>,
    registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<Vec<DatabaseCollection>, String> {
    let scope = workspace_ref
        .as_ref()
        .map(|reference| registry.resolve_workspace_ref(reference))
        .transpose()
        .map_err(|error| error.to_string())?;
    RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Database garbage collection is already running.".to_string())?;
    let guard = CollectionGuard;
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let mut results = vec![DatabaseCollection::from_result(
            "session",
            store.database_path(),
            store.garbage_collect(),
        )];
        if let Some(scope) = scope {
            let path = scope
                .runtime()
                .root()
                .join("Library")
                .join("Locus")
                .join("locus.db");
            if matches!(path.try_exists(), Ok(false)) {
                results.push(DatabaseCollection {
                    kind: "project",
                    path: path.to_string_lossy().into_owned(),
                    result: None,
                    error: None,
                    skipped: true,
                });
            } else {
                let asset_db = scope.runtime().core().asset_db();
                let result = (|| {
                    let db = asset_db.try_lock().map_err(|_| {
                        "Project database is busy; retry after indexing finishes.".to_string()
                    })?;
                    if let Some(db) = db.as_ref() {
                        crate::sqlite_maint::garbage_collect(&db.conn, &path)
                    } else {
                        collect_existing_project_database(&path)
                    }
                })();
                results.push(DatabaseCollection::from_result("project", &path, result));
            }
        }
        results
    })
    .await
    .map_err(|error| format!("Database garbage collection failed: {error}"))
}

fn collect_existing_project_database(path: &Path) -> Result<GarbageCollectionResult, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|error| format!("Failed to open existing project database: {error}"))?;
    conn.busy_timeout(std::time::Duration::from_millis(1500))
        .map_err(|error| error.to_string())?;
    crate::sqlite_maint::garbage_collect(&conn, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_keeps_old_project_schema_and_never_creates_missing_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locus.db");
        assert!(collect_existing_project_database(&path).is_err());
        assert!(!path.exists());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA user_version=1; CREATE TABLE legacy(data TEXT);
            INSERT INTO legacy VALUES('keep this data');",
        )
        .unwrap();
        drop(conn);
        collect_existing_project_database(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT data FROM legacy", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "keep this data"
        );
    }
}
