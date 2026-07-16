use rusqlite::Connection;

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
