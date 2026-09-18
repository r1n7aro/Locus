use rusqlite::Connection;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSpace {
    pub database_bytes: u64,
    pub wal_bytes: u64,
    pub logical_bytes: u64,
    pub free_page_bytes: u64,
}

impl DatabaseSpace {
    fn disk_bytes(&self) -> u64 {
        self.database_bytes.saturating_add(self.wal_bytes)
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GarbageCollectionResult {
    pub before: DatabaseSpace,
    pub after: DatabaseSpace,
    pub reclaimed_bytes: u64,
    pub warning: Option<String>,
}

fn read_u64_pragma(conn: &Connection, name: &str) -> Result<u64, String> {
    let value: i64 = conn
        .query_row(&format!("PRAGMA main.{name}"), [], |row| row.get(0))
        .map_err(|error| format!("Failed to read {name}: {error}"))?;
    u64::try_from(value).map_err(|_| format!("Invalid negative {name}: {value}"))
}

pub(crate) fn database_space(
    conn: &Connection,
    path: &std::path::Path,
) -> Result<DatabaseSpace, String> {
    let page_size = read_u64_pragma(conn, "page_size")?;
    let mut wal_path = path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let wal_bytes = match std::fs::metadata(&wal_path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(format!("Failed to measure WAL: {error}")),
    };
    Ok(DatabaseSpace {
        database_bytes: std::fs::metadata(path)
            .map_err(|error| format!("Failed to measure database: {error}"))?
            .len(),
        wal_bytes,
        logical_bytes: read_u64_pragma(conn, "page_count")?.saturating_mul(page_size),
        free_page_bytes: read_u64_pragma(conn, "freelist_count")?.saturating_mul(page_size),
    })
}

fn truncate_wal(conn: &Connection) -> Result<(), String> {
    let busy: i64 = conn
        .query_row("PRAGMA main.wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .map_err(|error| format!("Failed to checkpoint WAL: {error}"))?;
    if busy != 0 {
        return Err(
            "Database is busy; retry garbage collection when other readers or writers finish."
                .into(),
        );
    }
    Ok(())
}

/// Explicit maintenance also repacks partially filled pages, even with an
/// empty freelist. Call under the owner's connection mutex, outside a transaction.
/// No rows, indexes, schema versions, or external files are removed.
pub(crate) fn garbage_collect(
    conn: &Connection,
    path: &std::path::Path,
) -> Result<GarbageCollectionResult, String> {
    if !conn.is_autocommit() {
        return Err("Database has an active transaction; retry after it finishes.".into());
    }
    let before = database_space(conn, path)?;
    // SQLite can require up to twice the original database size in additional
    // space for the temporary image and journal. Fail before starting a rewrite.
    let required = before.logical_bytes.saturating_mul(2);
    let available = fs4::available_space(path.parent().unwrap_or(path))
        .map_err(|error| format!("Failed to check database disk space: {error}"))?;
    if available < required {
        return Err(format!(
            "Insufficient disk space for database compaction: need up to {required} bytes, {available} available."
        ));
    }
    truncate_wal(conn)?;
    let temp_store = read_u64_pragma(conn, "temp_store")?;
    conn.execute_batch("PRAGMA temp_store=FILE;")
        .map_err(|error| format!("Failed to configure temporary storage: {error}"))?;
    let vacuum = conn
        .execute_batch("VACUUM main")
        .map_err(|error| format!("Failed to compact database: {error}"));
    let restore = conn
        .execute_batch(&format!("PRAGMA temp_store={temp_store}"))
        .map_err(|error| format!("Failed to restore temporary storage: {error}"));
    vacuum?;
    let checkpoint = truncate_wal(conn);
    let after = database_space(conn, path)?;
    Ok(GarbageCollectionResult {
        reclaimed_bytes: before.disk_bytes().saturating_sub(after.disk_bytes()),
        before,
        after,
        warning: checkpoint.err().or_else(|| restore.err()),
    })
}

/// Default thresholds for [`vacuum_if_fragmented`]: reclaim only when at
/// least 16 MB AND a quarter of the file is dead freelist pages, so routine
/// churn never triggers a rewrite.
pub(crate) const VACUUM_MIN_FREE_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const VACUUM_MIN_FREE_RATIO: f64 = 0.25;

/// Run `VACUUM` when the database file is dominated by freelist pages.
///
/// None of Locus's databases enable auto_vacuum, so SQLite keeps the file at
/// its high-water mark forever: bulk deletions (removed sessions, wiped
/// knowledge chunks/embeddings) only move pages onto the freelist. Below the
/// thresholds this costs three PRAGMA lookups and does nothing.
///
/// The caller must not hold an open transaction on `conn`. Returns the
/// number of bytes released back to the filesystem, or `None` when skipped.
pub(crate) fn vacuum_if_fragmented(
    conn: &Connection,
    min_free_bytes: u64,
    min_free_ratio: f64,
) -> Result<Option<u64>, String> {
    let read_pragma = |name: &str| -> Result<i64, String> {
        conn.query_row(&format!("PRAGMA {}", name), [], |row| row.get(0))
            .map_err(|e| format!("Failed to read {}: {}", name, e))
    };

    let page_size = read_pragma("page_size")?;
    let page_count = read_pragma("page_count")?;
    let freelist_count = read_pragma("freelist_count")?;
    if page_size <= 0 || page_count <= 0 || freelist_count <= 0 {
        return Ok(None);
    }

    let free_bytes = (freelist_count as u64).saturating_mul(page_size as u64);
    let free_ratio = freelist_count as f64 / page_count as f64;
    if free_bytes < min_free_bytes || free_ratio < min_free_ratio {
        return Ok(None);
    }

    // VACUUM materializes the compacted copy in a temporary database first;
    // with temp_store=MEMORY that copy would be built in RAM, so force
    // file-backed temp storage for the duration.
    let temp_store = read_pragma("temp_store")?;
    if temp_store == 2 {
        conn.execute_batch("PRAGMA temp_store=FILE;")
            .map_err(|e| format!("Failed to switch temp_store for VACUUM: {}", e))?;
    }
    let vacuum_result = conn
        .execute_batch("VACUUM")
        .map_err(|e| format!("Failed to VACUUM: {}", e));
    if temp_store == 2 {
        let _ = conn.execute_batch("PRAGMA temp_store=MEMORY;");
    }
    vacuum_result?;

    // In WAL mode the rewritten image lands in the -wal file, which has its
    // own high-water mark; truncate it now that everything is checkpointed.
    // Returns a status row (and is a no-op) on non-WAL databases.
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_row| Ok(()));

    let page_count_after = read_pragma("page_count").unwrap_or(page_count);
    let freed_pages = (page_count - page_count_after).max(0) as u64;
    Ok(Some(freed_pages.saturating_mul(page_size as u64)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn file_size(path: &std::path::Path) -> u64 {
        std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
    }

    #[test]
    fn manual_collection_preserves_rows_schema_and_fts_rowids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version=13;
            CREATE TABLE objects (object_key TEXT PRIMARY KEY, name TEXT NOT NULL);
            CREATE VIRTUAL TABLE search USING fts5(name, content='', contentless_delete=1);
            INSERT INTO objects(rowid, object_key, name) VALUES (10, 'a', 'Player'), (30, 'b', 'Camera');
            INSERT INTO search(rowid, name) SELECT rowid, name FROM objects;
            CREATE VIEW names AS SELECT name FROM objects;
            CREATE TRIGGER keep_names AFTER UPDATE ON objects BEGIN SELECT 1; END;")
            .unwrap();
        seed_and_delete(&conn);
        let schema = || {
            conn.prepare("SELECT type, name, sql FROM sqlite_schema ORDER BY name")
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let before_schema = schema();
        let result = garbage_collect(&conn, &path).unwrap();
        assert!(result.reclaimed_bytes > 0);
        assert_eq!(result.after.free_page_bytes, 0);
        assert_eq!(
            result.reclaimed_bytes,
            result.before.disk_bytes() - result.after.disk_bytes()
        );
        assert_eq!(schema(), before_schema);
        assert_eq!(read_u64_pragma(&conn, "user_version").unwrap(), 13);
        let matches: (i64, String, String) = conn.query_row(
            "SELECT o.rowid, o.object_key, o.name FROM objects o JOIN search s ON o.rowid=s.rowid WHERE search MATCH 'Camera'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        assert_eq!(matches, (30, "b".into(), "Camera".into()));
        assert_eq!(
            conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert!(garbage_collect(&conn, &path).unwrap().warning.is_none());
    }

    #[test]
    fn manual_collection_repacks_partial_pages_with_no_freelist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE items(id INTEGER PRIMARY KEY, data TEXT);
            WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<2000)
            INSERT INTO items SELECT x, printf('%0200d', x) FROM n;
            DELETE FROM items WHERE id%2=0;
            CREATE TABLE retained_metadata(value TEXT);",
        )
        .unwrap();
        assert_eq!(read_u64_pragma(&conn, "freelist_count").unwrap(), 0);
        let result = garbage_collect(&conn, &path).unwrap();
        assert!(result.reclaimed_bytes > 0);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM items", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1000
        );
    }

    #[test]
    fn manual_collection_truncates_wal_and_reports_physical_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        seed_and_delete(&conn);
        conn.execute("INSERT INTO blobs VALUES (100, X'010203')", [])
            .unwrap();
        let before = database_space(&conn, &path).unwrap();
        let result = garbage_collect(&conn, &path).unwrap();
        assert!(before.wal_bytes > 0);
        assert_eq!(result.after.wal_bytes, 0);
        assert!(result.warning.is_none());
        assert_eq!(
            result.reclaimed_bytes,
            before.disk_bytes() - result.after.disk_bytes()
        );
        assert_eq!(
            conn.query_row("SELECT data FROM blobs WHERE id=100", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn manual_collection_refuses_transactions_and_busy_wal_without_losing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("busy.db");
        let conn = Connection::open(&path).unwrap();
        conn.busy_timeout(std::time::Duration::ZERO).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE items(id INTEGER PRIMARY KEY);
            INSERT INTO items VALUES(1); BEGIN;",
        )
        .unwrap();
        assert!(garbage_collect(&conn, &path)
            .unwrap_err()
            .contains("active transaction"));
        conn.execute_batch("ROLLBACK").unwrap();
        let reader = Connection::open(&path).unwrap();
        reader.execute_batch("BEGIN; SELECT * FROM items;").unwrap();
        conn.execute_batch("INSERT INTO items VALUES(2)").unwrap();
        assert!(garbage_collect(&conn, &path).unwrap_err().contains("busy"));
        reader.execute_batch("ROLLBACK").unwrap();
        garbage_collect(&conn, &path).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM items", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    /// Insert ~4 MB of blobs and delete them all, leaving the file at its
    /// high-water mark with a nearly-full freelist.
    fn seed_and_delete(conn: &Connection) {
        conn.execute_batch("CREATE TABLE IF NOT EXISTS blobs (id INTEGER PRIMARY KEY, data BLOB)")
            .expect("create table");
        let payload = vec![0u8; 64 * 1024];
        for id in 0..64 {
            conn.execute(
                "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
                params![id, payload],
            )
            .expect("insert blob");
        }
        conn.execute("DELETE FROM blobs", []).expect("delete blobs");
    }

    #[test]
    fn vacuums_when_freelist_crosses_thresholds() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("frag.db");
        let conn = Connection::open(&path).expect("open db");
        seed_and_delete(&conn);

        let before = file_size(&path);
        let freed = vacuum_if_fragmented(&conn, 1024 * 1024, 0.25).expect("vacuum check");
        assert!(freed.is_some_and(|bytes| bytes > 0), "expected a vacuum");
        let after = file_size(&path);
        assert!(
            after < before,
            "file should shrink: before={} after={}",
            before,
            after
        );

        let count: i64 = conn
            .query_row("SELECT count(*) FROM blobs", [], |row| row.get(0))
            .expect("db stays usable");
        assert_eq!(count, 0);
    }

    #[test]
    fn skips_below_thresholds() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let conn = Connection::open(dir.path().join("clean.db")).expect("open db");
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            .expect("create table");
        // Fresh database: no freelist at all.
        assert!(vacuum_if_fragmented(&conn, 1024 * 1024, 0.25)
            .expect("vacuum check")
            .is_none());

        // Fragmented but under the absolute floor: ratio alone must not trip.
        seed_and_delete(&conn);
        assert!(vacuum_if_fragmented(&conn, u64::MAX, 0.25)
            .expect("vacuum check")
            .is_none());
    }

    #[test]
    fn restores_memory_temp_store_after_vacuum() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let conn = Connection::open(dir.path().join("mem.db")).expect("open db");
        conn.execute_batch("PRAGMA temp_store=MEMORY;")
            .expect("set temp_store");
        seed_and_delete(&conn);

        assert!(vacuum_if_fragmented(&conn, 1024, 0.01)
            .expect("vacuum check")
            .is_some());
        let temp_store: i64 = conn
            .query_row("PRAGMA temp_store", [], |row| row.get(0))
            .expect("read temp_store");
        assert_eq!(temp_store, 2, "temp_store=MEMORY must be restored");
    }
}
