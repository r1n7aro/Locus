use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, Transaction};

use super::types::*;

#[allow(dead_code)]
const CURRENT_PARSER_VERSION: u32 = 1;

/// Schema version. Bump on any incompatible asset-table schema change. Mismatch
/// at `open_db` time triggers a full DB delete + rebuild — `locus.db` is a
/// pure cache and we never migrate it.
pub const ASSET_DB_VERSION: u32 = 8;

pub(crate) fn db_path(project_root: &Path) -> PathBuf {
    project_root.join("Library").join("Locus").join("locus.db")
}

pub(crate) fn delete_db_files(db_path: &Path) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
}

pub(crate) fn read_user_version(conn: &Connection) -> Result<u32, String> {
    conn.query_row("PRAGMA user_version", [], |row| {
        row.get::<_, i64>(0).map(|v| v as u32)
    })
    .map_err(|e| format!("Failed to read user_version: {}", e))
}

pub(crate) fn configure_connection(conn: &Connection) -> Result<(), String> {
    // WAL + reduced fsync as before, plus tuning for the big DbWrite
    // transaction path: temp_store=MEMORY keeps sort/hash spills off disk,
    // cache_size bumps the page cache to 64 MB (negative = KB), mmap_size
    // lets SQLite mmap 256 MB of the DB file to skip a layer of syscalls.
    // All five are per-connection and reversible.
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA busy_timeout=5000;
         PRAGMA synchronous=NORMAL;
         PRAGMA temp_store=MEMORY;
         PRAGMA cache_size=-65536;
         PRAGMA mmap_size=268435456;",
    )
    .map_err(|e| format!("Failed to set pragmas: {}", e))
}

pub fn open_db(project_root: &Path) -> Result<Connection, String> {
    let db_dir = project_root.join("Library").join("Locus");
    std::fs::create_dir_all(&db_dir).map_err(|e| format!("Failed to create db dir: {}", e))?;

    let db_path = db_path(project_root);

    // Probe schema version. If it doesn't match, blow the DB away — we never
    // migrate. NOTE: callers MUST drop any prior `AssetDb` (and thus its
    // SQLite Connection) before invoking `open_db`, otherwise `remove_file`
    // fails on Windows due to the file lock. See `commands/ref_graph.rs`.
    if db_path.exists() {
        let needs_rebuild = match Connection::open(&db_path) {
            Ok(probe) => read_user_version(&probe)
                .map(|v| v != ASSET_DB_VERSION)
                .unwrap_or(true),
            Err(_) => true,
        };
        if needs_rebuild {
            delete_db_files(&db_path);
            eprintln!("[AssetDb] schema version mismatch — deleted locus.db, rebuilding fresh");
        }
    }

    let conn = Connection::open(&db_path).map_err(|e| format!("Failed to open database: {}", e))?;

    configure_connection(&conn)?;

    create_tables(&conn)?;

    conn.execute_batch(&format!("PRAGMA user_version = {}", ASSET_DB_VERSION))
        .map_err(|e| format!("Failed to set user_version: {}", e))?;

    Ok(conn)
}

pub(crate) fn ensure_aux_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS asset_scan_metrics (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            duplicate_guid_groups INTEGER NOT NULL DEFAULT 0,
            duplicate_guid_paths INTEGER NOT NULL DEFAULT 0,
            duplicate_guid_assets_only INTEGER NOT NULL DEFAULT 0,
            duplicate_guid_packages_only INTEGER NOT NULL DEFAULT 0,
            duplicate_guid_cross_root INTEGER NOT NULL DEFAULT 0,
            parse_failure_count INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS linked_asset_roots (
            link_rel_path TEXT PRIMARY KEY,
            target_path TEXT NOT NULL
        );",
    )
    .map_err(|e| format!("Failed to create auxiliary asset tables: {}", e))
}

fn create_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS assets (
            guid BLOB PRIMARY KEY,
            path TEXT NOT NULL,
            ext TEXT NOT NULL,
            kind INTEGER NOT NULL,
            exists_on_disk INTEGER NOT NULL,
            mtime_ns INTEGER NOT NULL,
            size INTEGER NOT NULL,
            content_hash BLOB NOT NULL,
            meta_hash BLOB NOT NULL,
            parser_version INTEGER NOT NULL,
            -- v4 derived columns (Rust writes these on every insert/update)
            root INTEGER NOT NULL,
            path_lower TEXT NOT NULL,
            file_name_lower TEXT NOT NULL,
            stem_lower TEXT NOT NULL,
            script_class_name TEXT NOT NULL,
            script_class_lower TEXT NOT NULL,
            script_namespace_lower TEXT NOT NULL,
            script_full_name_lower TEXT NOT NULL,
            script_type_search TEXT NOT NULL,
            script_inheritance_search TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS edges (
            src_guid BLOB NOT NULL,
            dst_guid BLOB NOT NULL,
            dst_file_id INTEGER,
            class_id_hint INTEGER,
            field_hint TEXT,
            ref_path TEXT
        );

        CREATE TABLE IF NOT EXISTS files (
            path TEXT PRIMARY KEY,
            file_role INTEGER NOT NULL,
            mtime_ns INTEGER NOT NULL,
            size INTEGER NOT NULL,
            hash128 BLOB NOT NULL,
            owner_guid BLOB
        );

        CREATE TABLE IF NOT EXISTS script_inheritance_terms (
            term TEXT NOT NULL,
            script_guid BLOB NOT NULL,
            PRIMARY KEY(term, script_guid)
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_assets_path ON assets(path);
        CREATE INDEX IF NOT EXISTS idx_assets_kind_stem
            ON assets(exists_on_disk, kind, stem_lower);
        CREATE INDEX IF NOT EXISTS idx_assets_root_stem
            ON assets(exists_on_disk, root, stem_lower);
        CREATE INDEX IF NOT EXISTS idx_assets_root_pathlower
            ON assets(exists_on_disk, root, path_lower);
        CREATE INDEX IF NOT EXISTS idx_assets_pathlower
            ON assets(exists_on_disk, path_lower);
        CREATE INDEX IF NOT EXISTS idx_assets_script_class
            ON assets(exists_on_disk, kind, script_class_lower);
        CREATE INDEX IF NOT EXISTS idx_assets_script_full_name
            ON assets(exists_on_disk, kind, script_full_name_lower);
        CREATE INDEX IF NOT EXISTS idx_assets_script_ns_class
            ON assets(exists_on_disk, kind, script_namespace_lower, script_class_lower);
        CREATE INDEX IF NOT EXISTS idx_edges_src ON edges(src_guid);
        CREATE INDEX IF NOT EXISTS idx_edges_dst ON edges(dst_guid);
        CREATE INDEX IF NOT EXISTS idx_files_owner_guid ON files(owner_guid);
        CREATE INDEX IF NOT EXISTS idx_script_inheritance_terms_guid
            ON script_inheritance_terms(script_guid);

        -- FTS5 trigram virtual table. Only consumed by
        -- `search_assets_for_command` (the asset-page free-text path).
        -- The public `db::search_assets` predicate engine deliberately does
        -- NOT touch this table — its `n:` semantics stay filename-only.
        CREATE VIRTUAL TABLE IF NOT EXISTS asset_search_fts USING fts5(
            guid_hex UNINDEXED,
            stem,
            path,
            script_types,
            tokenize = 'trigram'
        );",
    )
    .map_err(|e| format!("Failed to create tables: {}", e))?;

    ensure_aux_tables(conn)
}

/// Compute the four derived search columns for a workspace-relative path.
/// Returns `(root_i32, path_lower, file_name_lower, stem_lower)`. All
/// lowercase strings are ASCII-only `to_ascii_lowercase` (Unity asset names
/// are virtually always ASCII; non-ASCII still works but is byte-compared).
pub(crate) fn derive_search_cols(path: &str) -> (i32, String, String, String) {
    let path_lower = path.to_ascii_lowercase();
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let file_name_lower = file_name.to_ascii_lowercase();
    let stem_lower = file_name_lower
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| file_name_lower.clone());
    let root = AssetRoot::from_rel_path(path) as i32;
    (root, path_lower, file_name_lower, stem_lower)
}

/// FTS5 sync helpers. Every write to `assets` MUST go through one of these
/// (or `clear_all_in_tx`) so the trigram index stays consistent. Centralising
/// the writes here means watcher.rs never has to know FTS5 exists.
pub(crate) mod asset_fts {
    use rusqlite::{params, Transaction};

    pub fn insert_row(
        tx: &Transaction,
        guid_hex: &str,
        stem_lower: &str,
        path_lower: &str,
        script_types: &str,
    ) -> Result<(), String> {
        tx.execute(
            "INSERT INTO asset_search_fts (guid_hex, stem, path, script_types) VALUES (?1, ?2, ?3, ?4)",
            params![guid_hex, stem_lower, path_lower, script_types],
        )
        .map_err(|e| format!("Failed to insert asset_search_fts row: {}", e))?;
        Ok(())
    }

    pub fn delete_by_guid(tx: &Transaction, guid_hex: &str) -> Result<(), String> {
        tx.execute(
            "DELETE FROM asset_search_fts WHERE guid_hex = ?1",
            params![guid_hex],
        )
        .map_err(|e| format!("Failed to delete asset_search_fts row: {}", e))?;
        Ok(())
    }

    pub fn clear_all(tx: &Transaction) -> Result<(), String> {
        // NOTE: FTS5's `'delete-all'` fast-wipe command is only valid on
        // contentless or external-content tables. `asset_search_fts` stores
        // its own content (`stem`, `path`, `script_types`), so we have to
        // fall back to a plain `DELETE FROM`. If full-scan clear latency
        // ever becomes a problem, the fix is to migrate the schema to
        // `content=''` rather than reintroduce `'delete-all'` here.
        tx.execute("DELETE FROM asset_search_fts", [])
            .map_err(|e| format!("Failed to clear asset_search_fts: {}", e))?;
        Ok(())
    }
}

/// Wipe all asset/file/edge/FTS rows inside an existing transaction. Used as
/// the first DML of `AssetDb::full_scan`'s rebuild path so the wipe and the
/// re-insert succeed or fail atomically.
pub fn clear_all_in_tx(tx: &Transaction) -> Result<(), String> {
    tx.execute_batch(
        "DELETE FROM edges;
         DELETE FROM assets;
         DELETE FROM files;
         DELETE FROM script_inheritance_terms;
         DELETE FROM asset_scan_metrics;
         DELETE FROM linked_asset_roots;",
    )
    .map_err(|e| format!("Failed to clear tables: {}", e))?;
    asset_fts::clear_all(tx)?;
    Ok(())
}

fn script_inheritance_terms(asset: &AssetNode) -> Vec<String> {
    if asset.kind != AssetKind::Script || !asset.exists_on_disk {
        return Vec::new();
    }

    let mut terms = Vec::new();
    let mut seen = HashSet::new();
    for term in asset.script_inheritance_search.split_whitespace() {
        let normalized = term.trim().to_ascii_lowercase();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        terms.push(normalized);
    }
    terms
}

fn insert_script_inheritance_terms(tx: &Transaction, asset: &AssetNode) -> Result<(), String> {
    let terms = script_inheritance_terms(asset);
    if terms.is_empty() {
        return Ok(());
    }

    let mut stmt = tx
        .prepare_cached(
            "INSERT OR IGNORE INTO script_inheritance_terms (term, script_guid)
             VALUES (?1, ?2)",
        )
        .map_err(|e| format!("Failed to prepare script inheritance term insert: {}", e))?;
    for term in terms {
        stmt.execute(params![term, asset.guid.as_slice()])
            .map_err(|e| format!("Failed to insert script inheritance term: {}", e))?;
    }
    Ok(())
}

fn delete_script_inheritance_terms(tx: &Transaction, guid: &Guid) -> Result<(), String> {
    tx.execute(
        "DELETE FROM script_inheritance_terms WHERE script_guid = ?1",
        params![guid.as_slice()],
    )
    .map_err(|e| format!("Failed to delete script inheritance terms: {}", e))?;
    Ok(())
}

/// Incremental updates are keyed by the new meta GUID. If a path is reimported
/// with a different GUID, remove the stale canonical row for that path so mtime
/// scans converge instead of requeueing the same asset forever.
fn delete_same_path_asset_conflicts(tx: &Transaction, asset: &AssetNode) -> Result<(), String> {
    let stale_guids = {
        let mut stmt = tx
            .prepare_cached("SELECT guid FROM assets WHERE path = ?1 AND guid != ?2")
            .map_err(|e| format!("Failed to prepare same-path asset conflict query: {}", e))?;
        let rows = stmt
            .query_map(params![asset.path.as_str(), asset.guid.as_slice()], |row| {
                let blob: Vec<u8> = row.get(0)?;
                Ok(blob_to_guid(&blob))
            })
            .map_err(|e| format!("Failed to query same-path asset conflicts: {}", e))?;

        let mut guids = Vec::new();
        for row in rows {
            guids.push(row.map_err(|e| format!("Failed to read same-path asset conflict: {}", e))?);
        }
        guids
    };

    if stale_guids.is_empty() {
        return Ok(());
    }

    let meta_path = format!("{}.meta", asset.path);
    for stale_guid in stale_guids {
        let stale_guid_hex = guid_to_hex(&stale_guid);
        tx.execute(
            "DELETE FROM edges WHERE src_guid = ?1",
            params![stale_guid.as_slice()],
        )
        .map_err(|e| format!("Failed to delete stale outgoing edges: {}", e))?;
        tx.execute(
            "DELETE FROM files
             WHERE owner_guid = ?1
               AND (path = ?2 OR path = ?3)",
            params![
                stale_guid.as_slice(),
                asset.path.as_str(),
                meta_path.as_str()
            ],
        )
        .map_err(|e| format!("Failed to delete stale same-path file bookkeeping: {}", e))?;
        tx.execute(
            "DELETE FROM assets WHERE guid = ?1",
            params![stale_guid.as_slice()],
        )
        .map_err(|e| format!("Failed to delete stale same-path asset: {}", e))?;
        delete_script_inheritance_terms(tx, &stale_guid)?;
        asset_fts::delete_by_guid(tx, &stale_guid_hex)?;
    }

    Ok(())
}

pub fn set_scan_metrics(
    tx: &Transaction,
    overview: &DuplicateGuidOverview,
    parse_failure_count: u64,
) -> Result<(), String> {
    tx.execute(
        "INSERT OR REPLACE INTO asset_scan_metrics
         (singleton, duplicate_guid_groups, duplicate_guid_paths,
          duplicate_guid_assets_only, duplicate_guid_packages_only, duplicate_guid_cross_root,
          parse_failure_count)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            overview.group_count as i64,
            overview.path_count as i64,
            overview.assets_only_groups as i64,
            overview.packages_only_groups as i64,
            overview.cross_root_groups as i64,
            parse_failure_count as i64,
        ],
    )
    .map_err(|e| format!("Failed to persist asset scan metrics: {}", e))?;
    Ok(())
}

pub fn replace_linked_asset_roots(
    tx: &Transaction,
    roots: &[LinkedAssetRoot],
) -> Result<(), String> {
    tx.execute("DELETE FROM linked_asset_roots", [])
        .map_err(|e| format!("Failed to clear linked asset roots: {}", e))?;

    if roots.is_empty() {
        return Ok(());
    }

    let mut stmt = tx
        .prepare(
            "INSERT OR REPLACE INTO linked_asset_roots
             (link_rel_path, target_path)
             VALUES (?1, ?2)",
        )
        .map_err(|e| format!("Failed to prepare linked asset root insert: {}", e))?;

    for root in roots {
        stmt.execute(params![
            root.link_rel_path.as_str(),
            root.target_path.to_string_lossy().as_ref(),
        ])
        .map_err(|e| {
            format!(
                "Failed to persist linked asset root {} -> {}: {}",
                root.link_rel_path,
                root.target_path.display(),
                e
            )
        })?;
    }

    Ok(())
}

pub fn get_linked_asset_roots(conn: &Connection) -> Result<Vec<LinkedAssetRoot>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT link_rel_path, target_path
             FROM linked_asset_roots
             ORDER BY link_rel_path ASC",
        )
        .map_err(|e| format!("Failed to prepare linked asset root query: {}", e))?;

    let rows = stmt
        .query_map([], |row| {
            let link_rel_path: String = row.get(0)?;
            let target_path: String = row.get(1)?;
            Ok(LinkedAssetRoot {
                link_rel_path,
                target_path: PathBuf::from(target_path),
            })
        })
        .map_err(|e| format!("Failed to query linked asset roots: {}", e))?;

    let mut roots = Vec::new();
    for row in rows {
        roots.push(row.map_err(|e| format!("Failed to read linked asset roots: {}", e))?);
    }

    Ok(roots)
}

pub fn get_duplicate_guid_overview(conn: &Connection) -> Result<DuplicateGuidOverview, String> {
    let mut stmt = conn
        .prepare(
            "SELECT duplicate_guid_groups, duplicate_guid_paths,
                    duplicate_guid_assets_only, duplicate_guid_packages_only,
                    duplicate_guid_cross_root
             FROM asset_scan_metrics
             WHERE singleton = 1",
        )
        .map_err(|e| format!("Failed to prepare duplicate GUID query: {}", e))?;

    match stmt.query_row([], |row| {
        Ok(DuplicateGuidOverview {
            group_count: row.get::<_, i64>(0)? as u64,
            path_count: row.get::<_, i64>(1)? as u64,
            assets_only_groups: row.get::<_, i64>(2)? as u64,
            packages_only_groups: row.get::<_, i64>(3)? as u64,
            cross_root_groups: row.get::<_, i64>(4)? as u64,
        })
    }) {
        Ok(overview) => Ok(overview),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(DuplicateGuidOverview::default()),
        Err(e) => Err(format!("Failed to read duplicate GUID overview: {}", e)),
    }
}

pub fn get_parse_failure_count(conn: &Connection) -> Result<u64, String> {
    let mut stmt = conn
        .prepare(
            "SELECT parse_failure_count
             FROM asset_scan_metrics
             WHERE singleton = 1",
        )
        .map_err(|e| format!("Failed to prepare parse failure count query: {}", e))?;

    match stmt.query_row([], |row| row.get::<_, i64>(0)) {
        Ok(count) => Ok(count as u64),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(e) => Err(format!("Failed to read parse failure count: {}", e)),
    }
}

#[derive(Debug, Clone)]
pub struct MissingReferenceCounts {
    pub broken_references: u64,
    pub missing_scripts: u64,
}

#[derive(Debug, Clone)]
pub struct MissingReferenceRow {
    pub src_path: String,
    pub src_kind: AssetKind,
    pub dst_guid: Guid,
    pub field_hint: Option<String>,
    pub ref_path: Option<String>,
}

pub fn get_missing_reference_counts(conn: &Connection) -> Result<MissingReferenceCounts, String> {
    let mut stmt = conn
        .prepare(
            "SELECT
                 SUM(CASE
                     WHEN dst.guid IS NULL
                      AND (e.field_hint IS NULL OR e.field_hint != 'm_Script')
                     THEN 1 ELSE 0 END),
                 SUM(CASE
                     WHEN dst.guid IS NULL
                      AND e.field_hint = 'm_Script'
                     THEN 1 ELSE 0 END)
             FROM edges e
             JOIN assets src
               ON src.guid = e.src_guid
              AND src.exists_on_disk = 1
             LEFT JOIN assets dst
               ON dst.guid = e.dst_guid
              AND dst.exists_on_disk = 1",
        )
        .map_err(|e| format!("Failed to prepare missing reference count query: {}", e))?;

    stmt.query_row([], |row| {
        Ok(MissingReferenceCounts {
            broken_references: row.get::<_, Option<i64>>(0)?.unwrap_or(0) as u64,
            missing_scripts: row.get::<_, Option<i64>>(1)?.unwrap_or(0) as u64,
        })
    })
    .map_err(|e| format!("Failed to read missing reference counts: {}", e))
}

pub fn get_missing_reference_rows(
    conn: &Connection,
    missing_scripts_only: bool,
) -> Result<Vec<MissingReferenceRow>, String> {
    let where_clause = if missing_scripts_only {
        "e.field_hint = 'm_Script'"
    } else {
        "(e.field_hint IS NULL OR e.field_hint != 'm_Script')"
    };

    let mut stmt = conn
        .prepare(&format!(
            "SELECT src.path, src.kind, e.dst_guid, e.field_hint, e.ref_path
             FROM edges e
             JOIN assets src
               ON src.guid = e.src_guid
              AND src.exists_on_disk = 1
             LEFT JOIN assets dst
               ON dst.guid = e.dst_guid
              AND dst.exists_on_disk = 1
             WHERE dst.guid IS NULL
               AND {}
             ORDER BY src.path, COALESCE(e.ref_path, ''), COALESCE(e.field_hint, ''), lower(hex(e.dst_guid))",
            where_clause
        ))
        .map_err(|e| format!("Failed to prepare missing reference detail query: {}", e))?;

    let rows = stmt
        .query_map([], |row| {
            let dst_blob: Vec<u8> = row.get(2)?;
            Ok(MissingReferenceRow {
                src_path: row.get(0)?,
                src_kind: AssetKind::from_i32(row.get(1)?),
                dst_guid: blob_to_guid(&dst_blob),
                field_hint: row.get(3)?,
                ref_path: row.get(4)?,
            })
        })
        .map_err(|e| format!("Failed to query missing reference detail rows: {}", e))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("Failed to read missing reference detail row: {}", e))?);
    }
    Ok(out)
}

/// Bulk insert asset rows + their FTS5 search index entries.
///
/// **CALLER CONTRACT**: this function does NOT pre-delete existing FTS rows
/// for each guid. It assumes the caller has just wiped `asset_search_fts`
/// (e.g. via `clear_all_in_tx`) — which is true for the full_scan rebuild
/// path. Doing per-row FTS deletes here would be O(n²): `guid_hex` is an
/// `UNINDEXED` column, so each delete scans the entire growing FTS table.
/// On a 16k-row Unity project that pushed `DbWrite` from a few seconds to
/// effectively hanging.
///
/// Incremental writes (single asset add/update) live in `atomic_update_asset`
/// and DO perform the FTS delete because their cardinality is 1.
pub fn batch_insert_assets(tx: &Transaction, assets: &[AssetNode]) -> Result<u64, String> {
    let mut count = 0u64;
    {
        let mut asset_stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO assets
                 (guid, path, ext, kind, exists_on_disk, mtime_ns, size,
                   content_hash, meta_hash, parser_version,
                   root, path_lower, file_name_lower, stem_lower,
                   script_class_name, script_class_lower, script_namespace_lower,
                   script_full_name_lower, script_type_search, script_inheritance_search)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                         ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            )
            .map_err(|e| format!("Failed to prepare asset insert: {}", e))?;
        let mut fts_stmt = tx
            .prepare(
                "INSERT INTO asset_search_fts (guid_hex, stem, path, script_types) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| format!("Failed to prepare fts insert: {}", e))?;

        for a in assets {
            let (root, path_lower, file_name_lower, stem_lower) = derive_search_cols(&a.path);
            asset_stmt
                .execute(params![
                    a.guid.as_slice(),
                    a.path,
                    a.ext,
                    a.kind as i32,
                    a.exists_on_disk as i32,
                    a.mtime_ns as i64,
                    a.size as i64,
                    a.content_hash.as_slice(),
                    a.meta_hash.as_slice(),
                    a.parser_version as i32,
                    root,
                    path_lower,
                    file_name_lower,
                    stem_lower,
                    a.script_class_name.as_deref().unwrap_or(""),
                    a.script_class_lower.as_str(),
                    a.script_namespace_lower.as_str(),
                    a.script_full_name_lower.as_str(),
                    a.script_type_search.as_str(),
                    a.script_inheritance_search.as_str(),
                ])
                .map_err(|e| format!("Failed to insert asset: {}", e))?;
            let guid_hex = guid_to_hex(&a.guid);
            fts_stmt
                .execute(params![
                    guid_hex,
                    stem_lower,
                    path_lower,
                    a.script_type_search.as_str(),
                ])
                .map_err(|e| format!("Failed to insert asset_search_fts row: {}", e))?;
            insert_script_inheritance_terms(tx, a)?;
            count += 1;
        }
    }
    Ok(count)
}

pub fn batch_insert_files(
    tx: &Transaction,
    files: &[(String, FileRole, u64, u64, [u8; 16], Option<Guid>)],
) -> Result<(), String> {
    let mut stmt = tx
        .prepare_cached(
            "INSERT OR REPLACE INTO files (path, file_role, mtime_ns, size, hash128, owner_guid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|e| format!("Failed to prepare file insert: {}", e))?;

    for (path, role, mtime, size, hash, owner) in files {
        let owner_slice: Option<&[u8]> = owner.as_ref().map(|g| g.as_slice());
        stmt.execute(params![
            path,
            *role as i32,
            *mtime as i64,
            *size as i64,
            hash.as_slice(),
            owner_slice,
        ])
        .map_err(|e| format!("Failed to insert file: {}", e))?;
    }
    Ok(())
}

pub fn batch_insert_edges(tx: &Transaction, edges: &[RefEdge]) -> Result<u64, String> {
    let mut stmt = tx
        .prepare_cached(
            "INSERT OR IGNORE INTO edges
             (src_guid, dst_guid, dst_file_id, class_id_hint, field_hint, ref_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|e| format!("Failed to prepare edge insert: {}", e))?;

    let mut count = 0u64;
    for e in edges {
        let rows = stmt
            .execute(params![
                e.src_guid.as_slice(),
                e.dst_guid.as_slice(),
                e.dst_file_id,
                e.class_id_hint,
                e.field_hint,
                e.ref_path,
            ])
            .map_err(|e| format!("Failed to insert edge: {}", e))?;
        count += rows as u64;
    }
    Ok(count)
}

pub fn get_direct_deps(conn: &Connection, guid: &Guid) -> Result<Vec<RefEdge>, String> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT src_guid, dst_guid, dst_file_id, class_id_hint, field_hint, ref_path
             FROM edges WHERE src_guid = ?1",
        )
        .map_err(|e| format!("Failed to prepare deps query: {}", e))?;

    read_edges(&mut stmt, guid)
}

pub fn get_direct_refs(conn: &Connection, guid: &Guid) -> Result<Vec<RefEdge>, String> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT src_guid, dst_guid, dst_file_id, class_id_hint, field_hint, ref_path
             FROM edges WHERE dst_guid = ?1",
        )
        .map_err(|e| format!("Failed to prepare refs query: {}", e))?;

    read_edges(&mut stmt, guid)
}

fn read_edges(stmt: &mut rusqlite::CachedStatement, guid: &Guid) -> Result<Vec<RefEdge>, String> {
    let rows = stmt
        .query_map(params![guid.as_slice()], |row| {
            let src: Vec<u8> = row.get(0)?;
            let dst: Vec<u8> = row.get(1)?;
            Ok((
                src,
                dst,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i32>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|e| format!("Failed to query edges: {}", e))?;

    let mut edges = Vec::new();
    for row in rows {
        let (src, dst, file_id, class_id, field, ref_path) =
            row.map_err(|e| format!("Failed to read edge: {}", e))?;
        edges.push(RefEdge {
            src_guid: blob_to_guid(&src),
            dst_guid: blob_to_guid(&dst),
            dst_file_id: file_id,
            class_id_hint: class_id,
            field_hint: field,
            ref_path,
        });
    }
    Ok(edges)
}

pub fn resolve_guid_by_path(conn: &Connection, path: &str) -> Result<Option<Guid>, String> {
    let result = conn.query_row(
        "SELECT guid FROM assets WHERE path = ?1",
        params![path],
        |row| {
            let blob: Vec<u8> = row.get(0)?;
            Ok(blob)
        },
    );

    match result {
        Ok(blob) => Ok(Some(blob_to_guid(&blob))),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to resolve guid: {}", e)),
    }
}

pub fn resolve_path_by_guid(conn: &Connection, guid: &Guid) -> Result<Option<String>, String> {
    let result = conn.query_row(
        "SELECT path FROM assets WHERE guid = ?1",
        params![guid.as_slice()],
        |row| row.get::<_, String>(0),
    );

    match result {
        Ok(path) => Ok(Some(path)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to resolve path: {}", e)),
    }
}

pub fn batch_resolve_paths(
    conn: &Connection,
    guids: &[Guid],
) -> Result<std::collections::HashMap<Guid, String>, String> {
    use std::collections::HashMap;

    if guids.is_empty() {
        return Ok(HashMap::new());
    }

    const BATCH_SIZE: usize = 500;
    let mut map = HashMap::with_capacity(guids.len());

    for chunk in guids.chunks(BATCH_SIZE) {
        let placeholders: String = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT guid, path FROM assets WHERE guid IN ({})",
            placeholders
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("Failed to prepare batch resolve: {}", e))?;

        let slices: Vec<&[u8]> = chunk.iter().map(|g| g.as_slice()).collect();
        let params_vec: Vec<&dyn rusqlite::types::ToSql> = slices
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        let rows = stmt
            .query_map(params_vec.as_slice(), |row| {
                let blob: Vec<u8> = row.get(0)?;
                let path: String = row.get(1)?;
                Ok((blob, path))
            })
            .map_err(|e| format!("Failed to batch resolve: {}", e))?;

        for row in rows {
            let (blob, path) = row.map_err(|e| format!("Failed to read row: {}", e))?;
            map.insert(blob_to_guid(&blob), path);
        }
    }

    Ok(map)
}

pub fn resolve_path_and_kind_by_guid(
    conn: &Connection,
    guid: &Guid,
) -> Result<Option<(String, AssetKind)>, String> {
    let result = conn.query_row(
        "SELECT path, kind FROM assets WHERE guid = ?1",
        params![guid.as_slice()],
        |row| {
            let path: String = row.get(0)?;
            let kind_i: i32 = row.get(1)?;
            Ok((path, kind_i))
        },
    );

    match result {
        Ok((path, kind_i)) => Ok(Some((path, AssetKind::from_i32(kind_i)))),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to resolve path and kind: {}", e)),
    }
}

pub fn walk_deps(conn: &Connection, root: &Guid, max_depth: u32) -> Result<Vec<Guid>, String> {
    bfs_walk(conn, root, max_depth, true)
}

pub fn walk_refs(conn: &Connection, root: &Guid, max_depth: u32) -> Result<Vec<Guid>, String> {
    bfs_walk(conn, root, max_depth, false)
}

fn bfs_walk(
    conn: &Connection,
    root: &Guid,
    max_depth: u32,
    forward: bool,
) -> Result<Vec<Guid>, String> {
    use std::collections::{HashSet, VecDeque};

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut result = Vec::new();

    visited.insert(*root);
    queue.push_back((*root, 0u32));

    while let Some((current, depth)) = queue.pop_front() {
        if depth > 0 {
            result.push(current);
        }
        if depth >= max_depth {
            continue;
        }

        let neighbors = if forward {
            get_direct_deps(conn, &current)?
        } else {
            get_direct_refs(conn, &current)?
        };

        for edge in &neighbors {
            let next = if forward {
                edge.dst_guid
            } else {
                edge.src_guid
            };
            if visited.insert(next) {
                queue.push_back((next, depth + 1));
            }
        }
    }

    Ok(result)
}

#[derive(Debug)]
pub enum SearchPredicate {
    Type(Vec<AssetKind>),
    NameExact(Vec<String>),
    NamePrefix(Vec<String>),
    NameSuffix(Vec<String>),
    NameContains(Vec<String>),
    Under(String),
    GuidExact(Guid),
    /// Restrict results to one or more workspace roots. Not parseable from
    /// the public DSL — only injected programmatically by the command layer.
    RootIn(Vec<AssetRoot>),
}

#[derive(Debug, Clone)]
pub struct AssetRow {
    pub tp: Option<String>,
    pub n: Option<String>,
    pub p: Option<String>,
    pub guid: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub total: u64,
    pub rows: Vec<AssetRow>,
}

fn format_tool_asset_type(kind: AssetKind, script_class_name: Option<&str>) -> String {
    let kind_name = kind.camel_str();
    if kind == AssetKind::GenericAsset {
        if let Some(class_name) = script_class_name.filter(|name| !name.is_empty()) {
            return format!("{}({})", class_name, kind_name);
        }
    }
    kind_name.to_string()
}

#[derive(Debug, Clone)]
pub struct StoredScriptMetadata {
    pub class_name: String,
    pub class_name_lower: String,
    pub namespace_lower: String,
    pub full_name_lower: String,
    pub type_search_lower: String,
    pub inheritance_search_lower: String,
}

impl StoredScriptMetadata {
    pub fn inherits_scriptable_object(&self) -> bool {
        self.type_search_lower
            .split_whitespace()
            .any(|term| term == "scriptableobject")
    }

    pub fn cascade_lookup_term(&self) -> &str {
        if self.full_name_lower.is_empty() {
            self.class_name_lower.as_str()
        } else {
            self.full_name_lower.as_str()
        }
    }
}

fn map_stored_script_metadata_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<StoredScriptMetadata> {
    Ok(StoredScriptMetadata {
        class_name: row.get(0)?,
        class_name_lower: row.get(1)?,
        namespace_lower: row.get(2)?,
        full_name_lower: row.get(3)?,
        type_search_lower: row.get(4)?,
        inheritance_search_lower: row.get(5)?,
    })
}

fn parse_asset_type(s: &str) -> Option<AssetKind> {
    match s.to_lowercase().as_str() {
        "scene" => Some(AssetKind::Scene),
        "prefab" => Some(AssetKind::Prefab),
        "genericasset" | "scriptableobject" | "asset" => Some(AssetKind::GenericAsset),
        "material" | "mat" => Some(AssetKind::Material),
        "animation" | "anim" => Some(AssetKind::Animation),
        "animatorcontroller" | "controller" => Some(AssetKind::Controller),
        "otheryaml" => Some(AssetKind::OtherYaml),
        "metaonly" => Some(AssetKind::MetaOnly),
        "script" | "cs" => Some(AssetKind::Script),
        "texture" | "tex" | "image" => Some(AssetKind::Texture),
        "audio" | "sound" => Some(AssetKind::Audio),
        "shader" => Some(AssetKind::Shader),
        "model" | "mesh" | "fbx" => Some(AssetKind::Model),
        _ => None,
    }
}

fn extract_filename(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) => &name[..i],
        None => name,
    }
}

fn is_query_escape_char(ch: char) -> bool {
    ch == '"' || ch == '\\' || ch == '|' || ch.is_whitespace()
}

fn split_query_tokens(q: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut chars = q.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                if is_query_escape_char(next) {
                    current.push(ch);
                    current.push(next);
                    chars.next();
                    continue;
                }
            }
            current.push(ch);
            continue;
        }

        if ch == '"' {
            in_quote = !in_quote;
            current.push(ch);
            continue;
        }

        if ch.is_whitespace() && !in_quote {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }

        current.push(ch);
    }

    if in_quote {
        return Err(
            "Unclosed quote in query. Use double quotes around values with spaces, e.g. n:\"Point Light\"."
                .to_string(),
        );
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

fn split_query_tokens_lossy(q: &str) -> Vec<String> {
    split_query_tokens(q).unwrap_or_else(|_| q.split_whitespace().map(|s| s.to_string()).collect())
}

fn unquote_query_value(raw: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut in_quote = false;
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                if is_query_escape_char(next) {
                    out.push(next);
                    chars.next();
                    continue;
                }
            }
            out.push(ch);
            continue;
        }

        if ch == '"' {
            in_quote = !in_quote;
            continue;
        }

        out.push(ch);
    }

    if in_quote {
        return Err(
            "Unclosed quote in query. Use double quotes around values with spaces, e.g. n:\"Point Light\"."
                .to_string(),
        );
    }

    Ok(out)
}

fn split_or_values(raw: &str) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                if is_query_escape_char(next) {
                    current.push(ch);
                    current.push(next);
                    chars.next();
                    continue;
                }
            }
            current.push(ch);
            continue;
        }

        if ch == '"' {
            in_quote = !in_quote;
            current.push(ch);
            continue;
        }

        if ch == '|' && !in_quote {
            values.push(unquote_query_value(&current)?);
            current.clear();
            continue;
        }

        current.push(ch);
    }

    if in_quote {
        return Err(
            "Unclosed quote in query. Use double quotes around values with spaces, e.g. n:\"Point Light\"."
                .to_string(),
        );
    }

    values.push(unquote_query_value(&current)?);
    Ok(values)
}

fn split_or_values_lossy(raw: &str) -> Vec<String> {
    split_or_values(raw).unwrap_or_else(|_| raw.split('|').map(|s| s.to_string()).collect())
}

fn unquote_query_value_lossy(raw: &str) -> String {
    unquote_query_value(raw).unwrap_or_else(|_| raw.to_string())
}

pub fn parse_query(q: &str) -> Result<Vec<SearchPredicate>, String> {
    let mut predicates = Vec::new();

    for token in split_query_tokens(q)? {
        if let Some(rest) = token.strip_prefix("t:") {
            let mut kinds = Vec::new();
            for part in split_or_values(rest)? {
                match parse_asset_type(&part) {
                    Some(kind) => kinds.push(kind),
                    None => return Err(format!("Unknown asset type: '{}'. Valid types: scene, prefab, material, animation, animatorController, genericAsset, script, texture, audio, shader, model, otherYaml, metaOnly", part)),
                }
            }
            predicates.push(SearchPredicate::Type(kinds));
        } else if let Some(rest) = token.strip_prefix("n=") {
            predicates.push(SearchPredicate::NameExact(split_or_values(rest)?));
        } else if let Some(rest) = token.strip_prefix("n^") {
            predicates.push(SearchPredicate::NamePrefix(split_or_values(rest)?));
        } else if let Some(rest) = token.strip_prefix("n$") {
            predicates.push(SearchPredicate::NameSuffix(split_or_values(rest)?));
        } else if let Some(rest) = token.strip_prefix("n:") {
            predicates.push(SearchPredicate::NameContains(split_or_values(rest)?));
        } else if let Some(rest) = token.strip_prefix("under:") {
            predicates.push(SearchPredicate::Under(unquote_query_value(rest)?));
        } else if let Some(rest) = token.strip_prefix("guid:") {
            let guid = unquote_query_value(rest)?;
            match parse_guid_hex(&guid) {
                Some(g) => predicates.push(SearchPredicate::GuidExact(g)),
                None => return Err(format!("Invalid GUID: '{}' (expected 32 hex chars)", guid)),
            }
        } else {
            return Err(format!(
                "Unknown predicate: '{}'. Supported: t:, n=, n^, n$, n:, under:, guid:",
                token
            ));
        }
    }

    if predicates.is_empty() {
        return Err("Empty query. Provide at least one predicate (e.g. 't:prefab').".to_string());
    }

    Ok(predicates)
}

/// Output of `parse_query_lenient`. The `bare_terms` field carries any
/// un-prefixed tokens as individual space-separated terms. The asset-page
/// search treats these bare terms with AND semantics, matching the common
/// behaviour of tools like Everything / search engines. Explicit `n:foo`
/// stays as a `NameContains` predicate inside `predicates` and keeps strict
/// filename-only semantics.
pub struct LenientQuery {
    pub predicates: Vec<SearchPredicate>,
    pub bare_terms: Vec<String>,
}

/// Lenient version of `parse_query` used by the asset-page free-text path.
/// Same prefix grammar as `parse_query`, but bare tokens (anything not
/// prefixed by `t:` / `n=` / `n^` / `n$` / `n:` / `under:` / `guid:`) are
/// collected separately into `bare_terms` instead of being silently
/// converted to `NameContains` predicates. This keeps `n:foo` (filename
/// contains) and bare `foo bar` (free-text AND across path / type terms)
/// cleanly distinct so the command-layer router doesn't accidentally widen
/// explicit `n:` semantics to also search paths.
///
/// Empty input returns `LenientQuery { predicates: [], bare_terms: [] }`.
pub fn parse_query_lenient(q: &str) -> LenientQuery {
    let mut predicates: Vec<SearchPredicate> = Vec::new();
    let mut bare_terms: Vec<String> = Vec::new();
    for token in split_query_tokens_lossy(q) {
        if let Some(rest) = token.strip_prefix("t:") {
            let mut kinds = Vec::new();
            for part in split_or_values_lossy(rest) {
                if let Some(k) = parse_asset_type(&part) {
                    kinds.push(k);
                }
            }
            if !kinds.is_empty() {
                predicates.push(SearchPredicate::Type(kinds));
            }
        } else if let Some(rest) = token.strip_prefix("n=") {
            predicates.push(SearchPredicate::NameExact(split_or_values_lossy(rest)));
        } else if let Some(rest) = token.strip_prefix("n^") {
            predicates.push(SearchPredicate::NamePrefix(split_or_values_lossy(rest)));
        } else if let Some(rest) = token.strip_prefix("n$") {
            predicates.push(SearchPredicate::NameSuffix(split_or_values_lossy(rest)));
        } else if let Some(rest) = token.strip_prefix("n:") {
            // Explicit n: stays a structured filename-only predicate.
            predicates.push(SearchPredicate::NameContains(split_or_values_lossy(rest)));
        } else if let Some(rest) = token.strip_prefix("under:") {
            predicates.push(SearchPredicate::Under(unquote_query_value_lossy(rest)));
        } else if let Some(rest) = token.strip_prefix("guid:") {
            let guid = unquote_query_value_lossy(rest);
            if let Some(g) = parse_guid_hex(&guid) {
                predicates.push(SearchPredicate::GuidExact(g));
            }
        } else {
            // Bare token — kept out of `predicates` so the router can route
            // it through FTS / path-fragment search separately, with AND
            // semantics across terms.
            bare_terms.push(unquote_query_value_lossy(&token));
        }
    }
    LenientQuery {
        predicates,
        bare_terms,
    }
}

fn normalize_bare_terms(terms: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for term in terms {
        let lowered = term.trim().to_ascii_lowercase();
        if lowered.is_empty() || !seen.insert(lowered.clone()) {
            continue;
        }
        out.push(lowered);
    }
    out
}

fn build_bare_term_filter_where(
    bare_terms: &[String],
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    for term in bare_terms {
        clauses.push("(path_lower LIKE ? OR script_type_search LIKE ?)".to_string());
        let like_pat = format!("%{}%", term);
        params.push(Box::new(like_pat.clone()));
        params.push(Box::new(like_pat));
    }

    if clauses.is_empty() {
        ("1=1".to_string(), params)
    } else {
        (clauses.join(" AND "), params)
    }
}

fn build_bare_score_order(bare_terms: &[String]) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    if bare_terms.is_empty() {
        return ("path".to_string(), Vec::new());
    }

    let mut score_parts: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    for term in bare_terms {
        score_parts.push("(CASE WHEN script_class_lower = ? THEN 32 ELSE 0 END)".to_string());
        score_parts.push("(CASE WHEN script_class_lower LIKE ? THEN 16 ELSE 0 END)".to_string());
        score_parts.push("(CASE WHEN stem_lower = ? THEN 24 ELSE 0 END)".to_string());
        score_parts.push("(CASE WHEN stem_lower LIKE ? THEN 12 ELSE 0 END)".to_string());
        score_parts.push("(CASE WHEN script_type_search LIKE ? THEN 8 ELSE 0 END)".to_string());
        score_parts.push("(CASE WHEN file_name_lower LIKE ? THEN 4 ELSE 0 END)".to_string());

        let exact = term.clone();
        let prefix = format!("{}%", term);
        let contains = format!("%{}%", term);
        params.push(Box::new(exact.clone()));
        params.push(Box::new(prefix.clone()));
        params.push(Box::new(exact));
        params.push(Box::new(prefix));
        params.push(Box::new(contains.clone()));
        params.push(Box::new(contains));
    }

    (format!("({}) DESC, path", score_parts.join(" + ")), params)
}

fn build_fts_and_match_query(bare_terms: &[String]) -> Option<String> {
    let mut long_terms = Vec::new();
    for term in bare_terms {
        if term.chars().count() >= 3 {
            long_terms.push(format!("\"{}\"", term.replace('"', "\"\"")));
        }
    }
    if long_terms.is_empty() {
        None
    } else {
        Some(long_terms.join(" AND "))
    }
}

/// Result row returned by `search_assets_for_command`. Contains everything
/// the asset page needs to render a row without round-tripping the path
/// through Rust again.
#[derive(Debug, Clone)]
pub struct AssetSearchRowDb {
    pub guid: Guid,
    pub path: String,
    pub kind: AssetKind,
    pub root: AssetRoot,
    pub stem_lower: String,
    pub file_name_lower: String,
    pub script_class_name: Option<String>,
    pub script_class_lower: String,
    pub script_type_search: String,
}

pub fn get_stored_script_metadata(
    conn: &Connection,
    guid: &Guid,
) -> Result<Option<StoredScriptMetadata>, String> {
    let result = conn.query_row(
        "SELECT script_class_name, script_class_lower,
                script_namespace_lower, script_full_name_lower,
                script_type_search, script_inheritance_search
         FROM assets
         WHERE guid = ?1",
        params![guid.as_slice()],
        map_stored_script_metadata_row,
    );

    match result {
        Ok(meta) if meta.class_name.is_empty() => Ok(None),
        Ok(meta) => Ok(Some(meta)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("Failed to read stored script metadata: {}", e)),
    }
}

fn query_unique_script_metadata(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::types::ToSql],
    context: &str,
) -> Result<Option<StoredScriptMetadata>, String> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("Failed to prepare {}: {}", context, e))?;
    let rows = stmt
        .query_map(params, map_stored_script_metadata_row)
        .map_err(|e| format!("Failed to query {}: {}", context, e))?;

    let mut found: Option<StoredScriptMetadata> = None;
    for row in rows {
        let meta = row.map_err(|e| format!("Failed to read {} row: {}", context, e))?;
        if meta.class_name.is_empty() {
            continue;
        }
        if found.is_some() {
            return Ok(None);
        }
        found = Some(meta);
    }
    Ok(found)
}

pub fn get_stored_script_metadata_for_base_type(
    conn: &Connection,
    class_name: &str,
    preferred_namespace_lower: Option<&str>,
) -> Result<Option<StoredScriptMetadata>, String> {
    let class_lower = class_name.to_ascii_lowercase();
    if class_lower.is_empty() {
        return Ok(None);
    }

    if let Some(namespace_lower) = preferred_namespace_lower.filter(|ns| !ns.is_empty()) {
        let full_name_lower = format!("{}.{}", namespace_lower, class_lower);
        let kind = AssetKind::Script as i32;
        let params: [&dyn rusqlite::types::ToSql; 2] = [&kind, &full_name_lower];
        if let Some(meta) = query_unique_script_metadata(
            conn,
            "SELECT script_class_name, script_class_lower,
                    script_namespace_lower, script_full_name_lower,
                    script_type_search, script_inheritance_search
             FROM assets
             WHERE exists_on_disk = 1
               AND kind = ?1
               AND script_full_name_lower = ?2
             ORDER BY path",
            &params,
            "script metadata by full name",
        )? {
            return Ok(Some(meta));
        }
    }

    let kind = AssetKind::Script as i32;
    let params: [&dyn rusqlite::types::ToSql; 2] = [&kind, &class_lower];
    query_unique_script_metadata(
        conn,
        "SELECT script_class_name, script_class_lower,
                script_namespace_lower, script_full_name_lower,
                script_type_search, script_inheritance_search
         FROM assets
         WHERE exists_on_disk = 1
           AND kind = ?1
           AND script_class_lower = ?2
         ORDER BY path",
        &params,
        "script metadata by class name",
    )
}

pub fn find_asset_paths_referencing_guid(
    conn: &Connection,
    target_guid: &Guid,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT a.path
             FROM edges e
             JOIN assets a ON a.guid = e.src_guid
             WHERE e.dst_guid = ?1
               AND a.exists_on_disk = 1
             ORDER BY a.path",
        )
        .map_err(|e| format!("Failed to prepare asset referrer query: {}", e))?;

    let rows = stmt
        .query_map(params![target_guid.as_slice()], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| format!("Failed to query asset referrers: {}", e))?;

    let mut paths = Vec::new();
    for row in rows {
        paths.push(row.map_err(|e| format!("Failed to read asset referrer row: {}", e))?);
    }
    Ok(paths)
}

pub fn find_script_descendant_paths(
    conn: &Connection,
    lookup_terms: &[String],
    exclude_guid: &Guid,
) -> Result<Vec<String>, String> {
    let lowered: Vec<String> = lookup_terms
        .iter()
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    if lowered.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: Vec<&str> = lowered.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT DISTINCT a.path
         FROM script_inheritance_terms t
         JOIN assets a ON a.guid = t.script_guid
         WHERE t.term IN ({})
           AND a.exists_on_disk = 1
           AND a.kind = ?
           AND a.guid != ?
         ORDER BY a.path",
        placeholders.join(",")
    );

    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    for name in lowered {
        params_vec.push(Box::new(name));
    }
    params_vec.push(Box::new(AssetKind::Script as i32));
    params_vec.push(Box::new(exclude_guid.to_vec()));

    let refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare script descendant query: {}", e))?;
    let rows = stmt
        .query_map(refs.as_slice(), |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to query script descendants: {}", e))?;

    let mut paths = Vec::new();
    for row in rows {
        paths.push(row.map_err(|e| format!("Failed to read script descendant row: {}", e))?);
    }
    Ok(paths)
}

/// Asset-page free-text search. Pushes everything down to SQLite (B-tree
/// indexes for structured predicates, FTS5 trigram for substring path
/// search). Result is already sorted by score+path; caller just renders.
///
/// Routing:
/// - `guid:abcd...` → primary-key lookup, limit 1.
/// - Pure structured predicates (Type/RootIn/Under/NameExact/NamePrefix/
///   NameSuffix) → single SQL via `search_assets`-style WHERE.
/// - Has bare token(s) and length ≥ 3 → FTS5 MATCH on (stem, path) for the
///   first bare token, then re-filter by remaining structured predicates and
///   sort by relevance to the bare query.
/// - Has bare token(s) but length < 3 → stem_lower prefix range fallback
///   (FTS5 trigram doesn't index <3 char tokens).
pub fn search_assets_for_command(
    conn: &Connection,
    query: &str,
    roots: &[AssetRoot],
    limit: u32,
) -> Result<Vec<AssetSearchRowDb>, String> {
    let parsed = parse_query_lenient(query);
    let bare_terms = normalize_bare_terms(&parsed.bare_terms);
    if parsed.predicates.is_empty() && bare_terms.is_empty() {
        return Ok(Vec::new());
    }

    let predicates = parsed.predicates;

    // Build the structured predicate set: explicit DSL predicates + the
    // root injection from the command layer. NOTE: GuidExact lives here too
    // — we deliberately do NOT short-circuit on it, because that would
    // skip the RootIn / Type / MetaOnly filters and could surface assets
    // the user explicitly hid. The PK index still drives the lookup.
    let mut structured: Vec<&SearchPredicate> = predicates.iter().collect();
    let root_pred;
    if !roots.is_empty() {
        root_pred = SearchPredicate::RootIn(roots.to_vec());
        structured.push(&root_pred);
    }

    // Always exclude MetaOnly from the asset-page surface (matches the
    // walker fallback's `.meta` skip).
    let exclude_meta = format!(" AND kind != {}", AssetKind::MetaOnly as i32);
    let limit_i = limit as i64;

    // ── Branch A: no bare token → pure structured query ───────────────
    if bare_terms.is_empty() {
        let (where_sql, params_vec) = build_structured_where(&structured);
        let sql = format!(
            "SELECT guid, path, kind, root, stem_lower, file_name_lower,
                    script_class_name, script_class_lower, script_type_search
             FROM assets
             WHERE exists_on_disk = 1 AND {}{}
             ORDER BY path
             LIMIT ?",
            where_sql, exclude_meta
        );
        return run_select(conn, &sql, params_vec, limit_i);
    }

    let (struct_where, struct_params) = build_structured_where(&structured);
    let (bare_where, bare_params) = build_bare_term_filter_where(&bare_terms);
    let (score_order, score_params) = build_bare_score_order(&bare_terms);

    // ── Branch B: short query (<3 chars) → path_lower LIKE fallback ────
    // FTS5 trigram tokenizer doesn't index <3 char terms, so we can't use
    // it. Old walker did `path_lower.contains(query)`, so do the same in
    // SQL. The full table scan is fine: even on a 100k-row project a single
    // LIKE pass is sub-millisecond, and the LIMIT caps the work further.
    let Some(match_term) = build_fts_and_match_query(&bare_terms) else {
        let mut params_vec = struct_params;
        params_vec.extend(bare_params);
        params_vec.extend(score_params);
        let sql = format!(
            "SELECT guid, path, kind, root, stem_lower, file_name_lower,
                    script_class_name, script_class_lower, script_type_search
             FROM assets
             WHERE exists_on_disk = 1 AND {}{} AND {}
             ORDER BY {}
             LIMIT ?",
            struct_where, exclude_meta, bare_where, score_order
        );
        return run_select(conn, &sql, params_vec, limit_i);
    };

    // ── Branch C: FTS5 trigram substring search ────────────────────────
    // Done in TWO STEPS to keep the assets-side join honest. The original
    // single-statement form used `lower(hex(a.guid)) = fts.guid_hex`, which
    // wraps the primary key column in functions and forces SCAN assets.
    // Splitting lets us pass guids as plain BLOB params and hit the PK
    // autoindex.
    //
    // Step 1 — query FTS for guid_hex strings (small, capped at limit*4).
    let fts_buffer = (limit as i64).saturating_mul(4).max(50);
    let mut fts_stmt = conn
        .prepare(
            "SELECT guid_hex FROM asset_search_fts
             WHERE asset_search_fts MATCH ?1 LIMIT ?2",
        )
        .map_err(|e| format!("Prepare fts query failed: {}", e))?;
    let hex_rows = fts_stmt
        .query_map(params![match_term, fts_buffer], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| format!("FTS query failed: {}", e))?;
    let mut guid_blobs: Vec<Vec<u8>> = Vec::new();
    for h in hex_rows {
        let hex = h.map_err(|e| format!("FTS row read failed: {}", e))?;
        if let Some(g) = parse_guid_hex(&hex) {
            guid_blobs.push(g.to_vec());
        }
    }
    drop(fts_stmt);

    if guid_blobs.is_empty() {
        return Ok(Vec::new());
    }

    // Step 2 — fetch full rows by guid IN (...) with the rest of the
    // structured WHERE clauses applied.
    let placeholders: Vec<&str> = guid_blobs.iter().map(|_| "?").collect();
    let guid_in = placeholders.join(",");

    // Bind order: guid IN params, then structured / bare-term params, then
    // ranking params, then LIMIT.
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    for g in guid_blobs {
        params_vec.push(Box::new(g));
    }
    params_vec.extend(struct_params);
    params_vec.extend(bare_params);
    params_vec.extend(score_params);

    let sql = format!(
        "SELECT guid, path, kind, root, stem_lower, file_name_lower,
                script_class_name, script_class_lower, script_type_search
         FROM assets
         WHERE guid IN ({}) AND exists_on_disk = 1 AND {}{} AND {}
         ORDER BY {}
         LIMIT ?",
        guid_in, struct_where, exclude_meta, bare_where, score_order
    );

    run_select(conn, &sql, params_vec, limit_i)
}

/// Build a `WHERE` clause fragment (without the `exists_on_disk = 1` prefix)
/// from a slice of borrowed predicates. Returns `("1=1", [])` if empty so
/// callers can always splice it in.
fn build_structured_where(
    predicates: &[&SearchPredicate],
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    for pred in predicates {
        match pred {
            SearchPredicate::Type(kinds) => {
                if kinds.len() == 1 {
                    clauses.push("kind = ?".to_string());
                    params.push(Box::new(kinds[0] as i32));
                } else if !kinds.is_empty() {
                    let placeholders: Vec<&str> = kinds.iter().map(|_| "?").collect();
                    clauses.push(format!("kind IN ({})", placeholders.join(",")));
                    for k in kinds {
                        params.push(Box::new(*k as i32));
                    }
                }
            }
            SearchPredicate::NameExact(names) => {
                if names.len() == 1 {
                    clauses.push("stem_lower = ?".to_string());
                    params.push(Box::new(names[0].to_ascii_lowercase()));
                } else {
                    let placeholders: Vec<&str> = names.iter().map(|_| "?").collect();
                    clauses.push(format!("stem_lower IN ({})", placeholders.join(",")));
                    for n in names {
                        params.push(Box::new(n.to_ascii_lowercase()));
                    }
                }
            }
            SearchPredicate::NamePrefix(prefixes) => {
                let parts: Vec<String> = prefixes
                    .iter()
                    .map(|_| "(stem_lower >= ? AND stem_lower < ?)".to_string())
                    .collect();
                clauses.push(if parts.len() == 1 {
                    parts[0].clone()
                } else {
                    format!("({})", parts.join(" OR "))
                });
                for p in prefixes {
                    let lo = p.to_ascii_lowercase();
                    let hi = next_prefix(&lo);
                    params.push(Box::new(lo));
                    params.push(Box::new(hi));
                }
            }
            SearchPredicate::NameSuffix(suffixes) => {
                let parts: Vec<String> = suffixes
                    .iter()
                    .map(|_| "stem_lower LIKE ?".to_string())
                    .collect();
                clauses.push(if parts.len() == 1 {
                    parts[0].clone()
                } else {
                    format!("({})", parts.join(" OR "))
                });
                for s in suffixes {
                    params.push(Box::new(format!("%{}", s.to_ascii_lowercase())));
                }
            }
            SearchPredicate::NameContains(substrs) => {
                // Multi-element NameContains (i.e. user typed `n:a|b`) — keep
                // filename-only semantics here too. The single-element bare
                // token case has already been pulled out as `bare_query`.
                let parts: Vec<String> = substrs
                    .iter()
                    .map(|_| "file_name_lower LIKE ?".to_string())
                    .collect();
                clauses.push(if parts.len() == 1 {
                    parts[0].clone()
                } else {
                    format!("({})", parts.join(" OR "))
                });
                for s in substrs {
                    params.push(Box::new(format!("%{}%", s.to_ascii_lowercase())));
                }
            }
            SearchPredicate::Under(path_prefix) => {
                let mut lo = path_prefix.trim_end_matches('/').to_ascii_lowercase();
                lo.push('/');
                let hi = next_prefix(&lo);
                clauses.push("(path_lower >= ? AND path_lower < ?)".to_string());
                params.push(Box::new(lo));
                params.push(Box::new(hi));
            }
            SearchPredicate::GuidExact(guid) => {
                clauses.push("guid = ?".to_string());
                params.push(Box::new(guid.to_vec()));
            }
            SearchPredicate::RootIn(roots) => {
                if roots.len() == 1 {
                    clauses.push("root = ?".to_string());
                    params.push(Box::new(roots[0] as i32));
                } else if !roots.is_empty() {
                    let placeholders: Vec<&str> = roots.iter().map(|_| "?").collect();
                    clauses.push(format!("root IN ({})", placeholders.join(",")));
                    for r in roots {
                        params.push(Box::new(*r as i32));
                    }
                }
            }
        }
    }

    if clauses.is_empty() {
        ("1=1".to_string(), params)
    } else {
        (clauses.join(" AND "), params)
    }
}

fn run_select(
    conn: &Connection,
    sql: &str,
    mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>>,
    append_limit: i64,
) -> Result<Vec<AssetSearchRowDb>, String> {
    if append_limit >= 0 {
        params_vec.push(Box::new(append_limit));
    }
    let refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("Prepare failed: {}\nSQL: {}", e, sql))?;
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            let guid_blob: Vec<u8> = row.get(0)?;
            let path: String = row.get(1)?;
            let kind_i: i32 = row.get(2)?;
            let root_i: i32 = row.get(3)?;
            let stem_lower: String = row.get(4)?;
            let file_name_lower: String = row.get(5)?;
            let script_class_name: String = row.get(6)?;
            let script_class_lower: String = row.get(7)?;
            let script_type_search: String = row.get(8)?;
            Ok(AssetSearchRowDb {
                guid: blob_to_guid(&guid_blob),
                path,
                kind: AssetKind::from_i32(kind_i),
                root: AssetRoot::from_i32(root_i),
                stem_lower,
                file_name_lower,
                script_class_name: (!script_class_name.is_empty()).then_some(script_class_name),
                script_class_lower,
                script_type_search,
            })
        })
        .map_err(|e| format!("Query failed: {}", e))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("Row read failed: {}", e))?);
    }
    Ok(out)
}

/// Compute the half-open upper bound for a prefix range query, i.e. the
/// smallest string that is strictly greater than every string starting with
/// `s`. ASCII fast path; if the last char would overflow we fall back to
/// appending a sentinel that sorts strictly after any normal lowercased text.
pub(crate) fn next_prefix(s: &str) -> String {
    if s.is_empty() {
        return "\u{FFFF}".to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = char::from_u32(last as u32 + 1) {
            // Avoid surrogate range; for ASCII this is always fine.
            if (last as u32) < 0xD7FF {
                let mut out: String = chars.into_iter().collect();
                out.push(next);
                return out;
            }
        }
    }
    format!("{}\u{FFFF}", s)
}

/// Public DSL search. Filename-only `n:` semantics — does NOT touch the FTS5
/// table. Used by `commands/ref_graph.rs::ref_graph_search` and the agent
/// tool surface, both of which expect stable, predictable predicate
/// behaviour. Free-text / path-fragment search lives in
/// `search_assets_for_command`.
pub fn search_assets(
    conn: &Connection,
    predicates: &[SearchPredicate],
    fields: &[String],
    limit: u32,
    offset: u64,
) -> Result<SearchResult, String> {
    let mut where_clauses: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    // exists_on_disk goes first so it lines up with the leading column of the
    // composite indexes.
    where_clauses.push("exists_on_disk = 1".to_string());

    for pred in predicates {
        match pred {
            SearchPredicate::Type(kinds) => {
                if kinds.len() == 1 {
                    where_clauses.push("kind = ?".to_string());
                    params_vec.push(Box::new(kinds[0] as i32));
                } else {
                    let placeholders: Vec<&str> = kinds.iter().map(|_| "?").collect();
                    where_clauses.push(format!("kind IN ({})", placeholders.join(",")));
                    for k in kinds {
                        params_vec.push(Box::new(*k as i32));
                    }
                }
            }
            SearchPredicate::NameExact(names) => {
                // stem_lower = ? (B-tree). Public DSL is filename-only; no
                // FTS path involvement.
                if names.len() == 1 {
                    where_clauses.push("stem_lower = ?".to_string());
                    params_vec.push(Box::new(names[0].to_ascii_lowercase()));
                } else {
                    let placeholders: Vec<&str> = names.iter().map(|_| "?").collect();
                    where_clauses.push(format!("stem_lower IN ({})", placeholders.join(",")));
                    for n in names {
                        params_vec.push(Box::new(n.to_ascii_lowercase()));
                    }
                }
            }
            SearchPredicate::NamePrefix(prefixes) => {
                // Range query on stem_lower. For multiple prefixes we OR them
                // together (each becomes its own range).
                let parts: Vec<String> = prefixes
                    .iter()
                    .map(|_| "(stem_lower >= ? AND stem_lower < ?)".to_string())
                    .collect();
                if parts.len() == 1 {
                    where_clauses.push(parts[0].clone());
                } else {
                    where_clauses.push(format!("({})", parts.join(" OR ")));
                }
                for p in prefixes {
                    let lo = p.to_ascii_lowercase();
                    let hi = next_prefix(&lo);
                    params_vec.push(Box::new(lo));
                    params_vec.push(Box::new(hi));
                }
            }
            SearchPredicate::NameSuffix(suffixes) => {
                // No leading-anchor index path; LIKE on stem_lower scans the
                // index but at least dodges the old `replace(...)` expression.
                let parts: Vec<String> = suffixes
                    .iter()
                    .map(|_| "stem_lower LIKE ?".to_string())
                    .collect();
                if parts.len() == 1 {
                    where_clauses.push(parts[0].clone());
                } else {
                    where_clauses.push(format!("({})", parts.join(" OR ")));
                }
                for s in suffixes {
                    params_vec.push(Box::new(format!("%{}", s.to_ascii_lowercase())));
                }
            }
            SearchPredicate::NameContains(substrs) => {
                // Filename-only contains. Public DSL semantics preserved
                // exactly — we don't reach into FTS or path_lower here.
                let parts: Vec<String> = substrs
                    .iter()
                    .map(|_| "file_name_lower LIKE ?".to_string())
                    .collect();
                if parts.len() == 1 {
                    where_clauses.push(parts[0].clone());
                } else {
                    where_clauses.push(format!("({})", parts.join(" OR ")));
                }
                for s in substrs {
                    params_vec.push(Box::new(format!("%{}%", s.to_ascii_lowercase())));
                }
            }
            SearchPredicate::Under(path_prefix) => {
                // Case-insensitive prefix range on path_lower (preserves the
                // old `LIKE 'X/%'` ASCII case-insensitive behaviour).
                let mut lo = path_prefix.trim_end_matches('/').to_ascii_lowercase();
                lo.push('/');
                let hi = next_prefix(&lo);
                where_clauses.push("(path_lower >= ? AND path_lower < ?)".to_string());
                params_vec.push(Box::new(lo));
                params_vec.push(Box::new(hi));
            }
            SearchPredicate::GuidExact(guid) => {
                where_clauses.push("guid = ?".to_string());
                params_vec.push(Box::new(guid.to_vec()));
            }
            SearchPredicate::RootIn(roots) => {
                // The public DSL doesn't expose `root:` parsing, but other
                // callers (e.g. command layer) may inject this predicate
                // programmatically. Honour it here too.
                if roots.len() == 1 {
                    where_clauses.push("root = ?".to_string());
                    params_vec.push(Box::new(roots[0] as i32));
                } else if !roots.is_empty() {
                    let placeholders: Vec<&str> = roots.iter().map(|_| "?").collect();
                    where_clauses.push(format!("root IN ({})", placeholders.join(",")));
                    for r in roots {
                        params_vec.push(Box::new(*r as i32));
                    }
                }
            }
        }
    }

    let where_sql = where_clauses.join(" AND ");

    let count_sql = format!("SELECT COUNT(*) FROM assets WHERE {}", where_sql);
    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let total: u64 = conn
        .query_row(&count_sql, params_refs.as_slice(), |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|e| format!("Count query failed: {}", e))? as u64;

    if limit == 0 {
        return Ok(SearchResult {
            total,
            rows: Vec::new(),
        });
    }

    let select_sql = format!(
        "SELECT guid, path, kind, script_class_name
         FROM assets
         WHERE {}
         ORDER BY path
         LIMIT ? OFFSET ?",
        where_sql
    );

    // Reuse the same params and append limit/offset.
    let mut all_params = params_vec;
    all_params.push(Box::new(limit as i64));
    all_params.push(Box::new(offset as i64));
    let all_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(|p| p.as_ref()).collect();

    let field_set: std::collections::HashSet<&str> = fields.iter().map(|s| s.as_str()).collect();
    let want_all = fields.is_empty();

    let mut stmt = conn
        .prepare(&select_sql)
        .map_err(|e| format!("Prepare failed: {}", e))?;

    let rows_iter = stmt
        .query_map(all_refs.as_slice(), |row| {
            let guid_blob: Vec<u8> = row.get(0)?;
            let path: String = row.get(1)?;
            let kind_i: i32 = row.get(2)?;
            let script_class_name: String = row.get(3)?;
            Ok((guid_blob, path, kind_i, script_class_name))
        })
        .map_err(|e| format!("Query failed: {}", e))?;

    let mut result_rows = Vec::new();
    for row_result in rows_iter {
        let (guid_blob, path, kind_i, script_class_name) =
            row_result.map_err(|e| format!("Row read failed: {}", e))?;

        let kind = AssetKind::from_i32(kind_i);
        let filename = extract_filename(&path);

        result_rows.push(AssetRow {
            tp: if field_set.contains("tp") {
                Some(format_tool_asset_type(
                    kind,
                    Some(script_class_name.as_str()),
                ))
            } else {
                None
            },
            n: if field_set.contains("n") {
                Some(filename.to_string())
            } else {
                None
            },
            p: if want_all || field_set.contains("p") {
                Some(path.clone())
            } else {
                None
            },
            guid: if field_set.contains("guid") {
                Some(guid_to_hex(&blob_to_guid(&guid_blob)))
            } else {
                None
            },
        });
    }

    Ok(SearchResult {
        total,
        rows: result_rows,
    })
}

#[allow(dead_code)]
pub fn delete_edges_by_src(conn: &Connection, guid: &Guid) -> Result<u64, String> {
    let rows = conn
        .execute(
            "DELETE FROM edges WHERE src_guid = ?1",
            params![guid.as_slice()],
        )
        .map_err(|e| format!("Failed to delete edges by src: {}", e))?;
    Ok(rows as u64)
}

#[allow(dead_code)]
pub fn upsert_file(
    conn: &Connection,
    path: &str,
    role: FileRole,
    mtime_ns: u64,
    size: u64,
    hash: &[u8; 16],
    owner_guid: Option<&Guid>,
) -> Result<(), String> {
    let owner_slice: Option<&[u8]> = owner_guid.map(|g| g.as_slice());
    conn.execute(
        "INSERT OR REPLACE INTO files (path, file_role, mtime_ns, size, hash128, owner_guid)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            path,
            role as i32,
            mtime_ns as i64,
            size as i64,
            hash.as_slice(),
            owner_slice
        ],
    )
    .map_err(|e| format!("Failed to upsert file: {}", e))?;
    Ok(())
}

pub fn delete_missing_asset_path(conn: &mut Connection, asset_path: &str) -> Result<bool, String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("Failed to begin tx: {}", e))?;

    let canonical_guid = match tx.query_row(
        "SELECT guid FROM assets WHERE path = ?1",
        params![asset_path],
        |row| row.get::<_, Vec<u8>>(0),
    ) {
        Ok(blob) => Some(blob_to_guid(&blob)),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(format!("Failed to resolve deleted asset path: {}", e)),
    };

    let meta_path = format!("{}.meta", asset_path);
    tx.execute(
        "DELETE FROM files WHERE path = ?1 OR path = ?2",
        params![asset_path, meta_path],
    )
    .map_err(|e| format!("Failed to delete file bookkeeping: {}", e))?;

    if let Some(guid) = canonical_guid {
        let g = guid.as_slice();
        tx.execute("DELETE FROM edges WHERE src_guid = ?1", params![g])
            .map_err(|e| format!("Failed to delete outgoing edges: {}", e))?;
        tx.execute("DELETE FROM files WHERE owner_guid = ?1", params![g])
            .map_err(|e| format!("Failed to delete files: {}", e))?;
        tx.execute("DELETE FROM assets WHERE guid = ?1", params![g])
            .map_err(|e| format!("Failed to delete asset: {}", e))?;
        delete_script_inheritance_terms(&tx, &guid)?;
        asset_fts::delete_by_guid(&tx, &guid_to_hex(&guid))?;
    }

    tx.commit()
        .map_err(|e| format!("Failed to commit deleted asset cleanup: {}", e))?;

    Ok(canonical_guid.is_some())
}

pub fn atomic_update_asset(
    conn: &mut Connection,
    asset: &AssetNode,
    new_edges: &[RefEdge],
    file_records: &[(String, FileRole, u64, u64, [u8; 16])],
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("Failed to begin tx: {}", e))?;

    delete_same_path_asset_conflicts(&tx, asset)?;

    tx.execute(
        "DELETE FROM edges WHERE src_guid = ?1",
        params![asset.guid.as_slice()],
    )
    .map_err(|e| format!("Failed to delete old edges: {}", e))?;

    let (root, path_lower, file_name_lower, stem_lower) = derive_search_cols(&asset.path);

    // Read the existing search keys BEFORE the upsert overwrites them.
    // Most watcher events are "content changed but file not renamed", in
    // which case stem_lower / path_lower are identical and the FTS5 trigram
    // rewrite below is pure waste. PK lookup, sub-millisecond.
    let old_search_keys: Option<(String, String, String)> = {
        let mut stmt = tx
            .prepare_cached(
                "SELECT stem_lower, path_lower, script_type_search FROM assets WHERE guid = ?1",
            )
            .map_err(|e| format!("Prepare FTS skip-check failed: {}", e))?;
        match stmt.query_row(params![asset.guid.as_slice()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) {
            Ok(pair) => Some(pair),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(format!("FTS skip-check query failed: {}", e)),
        }
    };

    let need_fts_update = match &old_search_keys {
        Some((old_stem, old_path, old_script_types)) => {
            old_stem != &stem_lower
                || old_path != &path_lower
                || old_script_types != &asset.script_type_search
        }
        None => true, // brand-new asset, must populate FTS
    };

    tx.execute(
        "INSERT OR REPLACE INTO assets
         (guid, path, ext, kind, exists_on_disk, mtime_ns, size,
          content_hash, meta_hash, parser_version,
          root, path_lower, file_name_lower, stem_lower,
          script_class_name, script_class_lower, script_namespace_lower,
          script_full_name_lower, script_type_search, script_inheritance_search)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                 ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            asset.guid.as_slice(),
            asset.path,
            asset.ext,
            asset.kind as i32,
            asset.exists_on_disk as i32,
            asset.mtime_ns as i64,
            asset.size as i64,
            asset.content_hash.as_slice(),
            asset.meta_hash.as_slice(),
            asset.parser_version as i32,
            root,
            path_lower,
            file_name_lower,
            stem_lower,
            asset.script_class_name.as_deref().unwrap_or(""),
            asset.script_class_lower.as_str(),
            asset.script_namespace_lower.as_str(),
            asset.script_full_name_lower.as_str(),
            asset.script_type_search.as_str(),
            asset.script_inheritance_search.as_str(),
        ],
    )
    .map_err(|e| format!("Failed to upsert asset: {}", e))?;

    if need_fts_update {
        let guid_hex = guid_to_hex(&asset.guid);
        asset_fts::delete_by_guid(&tx, &guid_hex)?;
        asset_fts::insert_row(
            &tx,
            &guid_hex,
            &stem_lower,
            &path_lower,
            asset.script_type_search.as_str(),
        )?;
    }

    delete_script_inheritance_terms(&tx, &asset.guid)?;
    insert_script_inheritance_terms(&tx, asset)?;

    {
        let mut stmt = tx
            .prepare_cached(
                "INSERT OR IGNORE INTO edges
                 (src_guid, dst_guid, dst_file_id, class_id_hint, field_hint, ref_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|e| format!("Failed to prepare edge insert: {}", e))?;

        for edge in new_edges {
            stmt.execute(params![
                edge.src_guid.as_slice(),
                edge.dst_guid.as_slice(),
                edge.dst_file_id,
                edge.class_id_hint,
                edge.field_hint,
                edge.ref_path,
            ])
            .map_err(|e| format!("Failed to insert edge: {}", e))?;
        }
    }

    for (path, role, mtime, size, hash) in file_records {
        tx.execute(
            "INSERT OR REPLACE INTO files (path, file_role, mtime_ns, size, hash128, owner_guid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                path,
                *role as i32,
                *mtime as i64,
                *size as i64,
                hash.as_slice(),
                asset.guid.as_slice(),
            ],
        )
        .map_err(|e| format!("Failed to upsert file: {}", e))?;
    }

    tx.commit()
        .map_err(|e| format!("Failed to commit: {}", e))?;

    Ok(())
}

pub fn get_stats(conn: &Connection) -> Result<(u64, u64), String> {
    let nodes: i64 = conn
        .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count assets: {}", e))?;
    let edges: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count edges: {}", e))?;
    Ok((nodes as u64, edges as u64))
}

pub fn get_asset_size_bytes(conn: &Connection) -> Result<u64, String> {
    let bytes: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(size), 0) FROM assets WHERE exists_on_disk = 1 AND kind != ?1",
            params![AssetKind::MetaOnly as i32],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to sum asset sizes: {}", e))?;
    Ok(bytes as u64)
}

pub(crate) fn get_file_count(conn: &Connection) -> Result<u64, String> {
    let files: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count files: {}", e))?;
    Ok(files as u64)
}

/// Returns `(kind_i32, count)` rows from `SELECT kind, COUNT(*) FROM assets GROUP BY kind`.
pub fn get_kind_counts(conn: &Connection) -> Result<Vec<(i32, u64)>, String> {
    let mut stmt = conn
        .prepare("SELECT kind, COUNT(*) FROM assets GROUP BY kind")
        .map_err(|e| format!("Failed to prepare kind count query: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            let kind: i32 = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((kind, count as u64))
        })
        .map_err(|e| format!("Failed to query kind counts: {}", e))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("Failed to read kind count row: {}", e))?);
    }
    Ok(out)
}

#[cfg(test)]
pub fn get_all_meta_asset_mtimes(conn: &Connection) -> Result<Vec<(String, u64)>, String> {
    let mut stmt = conn
        .prepare("SELECT path, mtime_ns FROM files WHERE file_role = ?1")
        .map_err(|e| format!("Failed to prepare meta-mtime query: {}", e))?;

    let rows = stmt
        .query_map(params![FileRole::Meta as i32], |row| {
            let path: String = row.get(0)?;
            let mtime: i64 = row.get(1)?;
            Ok((path, mtime as u64))
        })
        .map_err(|e| format!("Failed to query meta mtimes: {}", e))?;

    let mut result = Vec::new();
    for row in rows {
        let (path, mtime) = row.map_err(|e| format!("Failed to read meta mtime row: {}", e))?;
        result.push((
            path.strip_suffix(".meta").unwrap_or(&path).to_string(),
            mtime,
        ));
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct AssetMtimeRecord {
    pub path: String,
    pub mtime_ns: u64,
    pub size: u64,
    pub content_hash: [u8; 16],
    pub kind: AssetKind,
    pub exists_on_disk: bool,
}

pub fn get_all_asset_mtime_records(conn: &Connection) -> Result<Vec<AssetMtimeRecord>, String> {
    let mut stmt = conn
        .prepare("SELECT path, mtime_ns, size, content_hash, kind, exists_on_disk FROM assets")
        .map_err(|e| format!("Failed to prepare mtime record query: {}", e))?;

    let rows = stmt
        .query_map([], |row| {
            let path: String = row.get(0)?;
            let mtime: i64 = row.get(1)?;
            let size: i64 = row.get(2)?;
            let content_hash_blob: Vec<u8> = row.get(3)?;
            let kind: i32 = row.get(4)?;
            let exists: i32 = row.get(5)?;
            Ok(AssetMtimeRecord {
                path,
                mtime_ns: mtime as u64,
                size: size as u64,
                content_hash: blob_to_guid(&content_hash_blob),
                kind: AssetKind::from_i32(kind),
                exists_on_disk: exists != 0,
            })
        })
        .map_err(|e| format!("Failed to query mtime records: {}", e))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| format!("Failed to read mtime record row: {}", e))?);
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct FileMtimeRecord {
    pub path: String,
    pub file_role: FileRole,
    pub mtime_ns: u64,
    pub size: u64,
    pub hash128: [u8; 16],
}

pub fn get_all_file_mtime_records(conn: &Connection) -> Result<Vec<FileMtimeRecord>, String> {
    let mut stmt = conn
        .prepare("SELECT path, file_role, mtime_ns, size, hash128 FROM files")
        .map_err(|e| format!("Failed to prepare file mtime record query: {}", e))?;

    let rows = stmt
        .query_map([], |row| {
            let path: String = row.get(0)?;
            let role: i32 = row.get(1)?;
            let mtime: i64 = row.get(2)?;
            let size: i64 = row.get(3)?;
            let hash_blob: Vec<u8> = row.get(4)?;
            Ok(FileMtimeRecord {
                path,
                file_role: FileRole::from_i32(role),
                mtime_ns: mtime as u64,
                size: size as u64,
                hash128: blob_to_guid(&hash_blob),
            })
        })
        .map_err(|e| format!("Failed to query file mtime records: {}", e))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| format!("Failed to read file mtime record row: {}", e))?);
    }
    Ok(result)
}

#[allow(dead_code)]
pub fn get_all_asset_mtimes(conn: &Connection) -> Result<Vec<(String, u64)>, String> {
    let mut stmt = conn
        .prepare("SELECT path, mtime_ns FROM assets")
        .map_err(|e| format!("Failed to prepare mtime query: {}", e))?;

    let rows = stmt
        .query_map([], |row| {
            let path: String = row.get(0)?;
            let mtime: i64 = row.get(1)?;
            Ok((path, mtime as u64))
        })
        .map_err(|e| format!("Failed to query mtimes: {}", e))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| format!("Failed to read mtime row: {}", e))?);
    }
    Ok(result)
}

fn blob_to_guid(blob: &[u8]) -> Guid {
    let mut guid = [0u8; 16];
    let len = blob.len().min(16);
    guid[..len].copy_from_slice(&blob[..len]);
    guid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_asset(path: &str, kind: AssetKind, script_type_search: &str) -> AssetNode {
        let ext = path
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_string())
            .unwrap_or_default();
        AssetNode {
            guid: hash128(path.as_bytes()),
            path: path.to_string(),
            ext,
            kind,
            exists_on_disk: true,
            mtime_ns: 1,
            size: path.len() as u64,
            content_hash: hash128(format!("content:{path}").as_bytes()),
            meta_hash: hash128(format!("meta:{path}").as_bytes()),
            parser_version: 1,
            script_class_name: None,
            script_class_lower: String::new(),
            script_namespace_lower: String::new(),
            script_full_name_lower: String::new(),
            script_type_search: script_type_search.to_string(),
            script_inheritance_search: String::new(),
        }
    }

    fn test_script_asset(
        path: &str,
        class_name: &str,
        namespace_lower: &str,
        type_search: &str,
        inheritance_search: &str,
    ) -> AssetNode {
        let mut node = test_asset(path, AssetKind::Script, type_search);
        let class_lower = class_name.to_ascii_lowercase();
        node.script_class_name = Some(class_name.to_string());
        node.script_class_lower = class_lower.clone();
        node.script_namespace_lower = namespace_lower.to_string();
        node.script_full_name_lower = if namespace_lower.is_empty() {
            class_lower
        } else {
            format!("{}.{}", namespace_lower, class_name.to_ascii_lowercase())
        };
        node.script_inheritance_search = inheritance_search.to_string();
        node
    }

    fn seed_assets(conn: &mut Connection, assets: &[AssetNode]) {
        create_tables(conn).unwrap();
        let tx = conn.transaction().unwrap();
        batch_insert_assets(&tx, assets).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn parse_query_supports_quoted_or_name_values() {
        let parsed =
            parse_query(r#"t:prefab|material n:PointLight|"Point Light"|DefaultPointLight"#)
                .unwrap();

        assert_eq!(parsed.len(), 2);
        match &parsed[0] {
            SearchPredicate::Type(kinds) => {
                assert_eq!(kinds, &vec![AssetKind::Prefab, AssetKind::Material]);
            }
            other => panic!("expected type predicate, got {other:?}"),
        }
        match &parsed[1] {
            SearchPredicate::NameContains(names) => {
                assert_eq!(
                    names,
                    &vec![
                        "PointLight".to_string(),
                        "Point Light".to_string(),
                        "DefaultPointLight".to_string()
                    ]
                );
            }
            other => panic!("expected name predicate, got {other:?}"),
        }
    }

    #[test]
    fn parse_query_supports_quoted_under_values() {
        let parsed = parse_query(r#"under:"Assets/My Folder" n="Point Light""#).unwrap();

        assert_eq!(parsed.len(), 2);
        match &parsed[0] {
            SearchPredicate::Under(path) => assert_eq!(path, "Assets/My Folder"),
            other => panic!("expected under predicate, got {other:?}"),
        }
        match &parsed[1] {
            SearchPredicate::NameExact(names) => {
                assert_eq!(names, &vec!["Point Light".to_string()]);
            }
            other => panic!("expected exact name predicate, got {other:?}"),
        }
    }

    #[test]
    fn parse_query_supports_escaped_pipe_as_literal_value() {
        let parsed = parse_query(r#"n:Point\|Light|DefaultPointLight"#).unwrap();

        match &parsed[0] {
            SearchPredicate::NameContains(names) => {
                assert_eq!(
                    names,
                    &vec!["Point|Light".to_string(), "DefaultPointLight".to_string()]
                );
            }
            other => panic!("expected name predicate, got {other:?}"),
        }
    }

    #[test]
    fn parse_query_rejects_unclosed_quotes() {
        let err = parse_query(r#"n:"Point Light"#).unwrap_err();

        assert!(err.contains("Unclosed quote in query"));
    }

    #[test]
    fn search_assets_matches_quoted_name_with_space() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed_assets(
            &mut conn,
            &[
                test_asset("Assets/Lighting/Point Light.prefab", AssetKind::Prefab, ""),
                test_asset("Assets/Lighting/PointLight.prefab", AssetKind::Prefab, ""),
            ],
        );
        let predicates = parse_query(r#"t:prefab n:"Point Light""#).unwrap();

        let result = search_assets(&conn, &predicates, &["p".to_string()], 20, 0).unwrap();

        assert_eq!(result.total, 1);
        assert_eq!(
            result.rows[0].p.as_deref(),
            Some("Assets/Lighting/Point Light.prefab")
        );
    }

    #[test]
    fn get_all_meta_asset_mtimes_returns_duplicate_guid_aliases() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let guid = hash128(b"duplicate-guid");
        let tx = conn.transaction().unwrap();
        batch_insert_files(
            &tx,
            &[
                (
                    "Assets/Plugins/UniTask/Runtime/IUniTaskSource.cs.meta".to_string(),
                    FileRole::Meta,
                    1,
                    10,
                    hash128(b"a"),
                    Some(guid),
                ),
                (
                    "Assets/Plugins/YooAsset/Samples~/UniTask Sample/UniTask/Runtime/IUniTaskSource.cs.meta"
                        .to_string(),
                    FileRole::Meta,
                    2,
                    20,
                    hash128(b"b"),
                    Some(guid),
                ),
            ],
        )
        .unwrap();
        tx.commit().unwrap();

        let mut rows = get_all_meta_asset_mtimes(&conn).unwrap();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            rows,
            vec![
                ("Assets/Plugins/UniTask/Runtime/IUniTaskSource.cs".to_string(), 1),
                (
                    "Assets/Plugins/YooAsset/Samples~/UniTask Sample/UniTask/Runtime/IUniTaskSource.cs"
                        .to_string(),
                    2,
                ),
            ]
        );
    }

    #[test]
    fn delete_missing_asset_path_removes_alias_bookkeeping_only() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let guid = hash128(b"canonical-guid");
        let mut asset = test_asset("Assets/Game/Canonical.prefab", AssetKind::Prefab, "");
        asset.guid = guid;
        seed_assets(&mut conn, &[asset]);

        let tx = conn.transaction().unwrap();
        batch_insert_files(
            &tx,
            &[
                (
                    "Assets/Game/Canonical.prefab.meta".to_string(),
                    FileRole::Meta,
                    10,
                    10,
                    hash128(b"canonical-meta"),
                    Some(guid),
                ),
                (
                    "Assets/Game/Canonical.prefab".to_string(),
                    FileRole::YamlAsset,
                    11,
                    11,
                    hash128(b"canonical-content"),
                    Some(guid),
                ),
                (
                    "Assets/Game/Alias.prefab.meta".to_string(),
                    FileRole::Meta,
                    20,
                    20,
                    hash128(b"alias-meta"),
                    Some(guid),
                ),
                (
                    "Assets/Game/Alias.prefab".to_string(),
                    FileRole::YamlAsset,
                    21,
                    21,
                    hash128(b"alias-content"),
                    Some(guid),
                ),
            ],
        )
        .unwrap();
        tx.commit().unwrap();

        let removed_canonical =
            delete_missing_asset_path(&mut conn, "Assets/Game/Alias.prefab").unwrap();
        assert!(!removed_canonical);
        assert_eq!(
            resolve_guid_by_path(&conn, "Assets/Game/Canonical.prefab").unwrap(),
            Some(guid)
        );

        let mut meta_rows = get_all_meta_asset_mtimes(&conn).unwrap();
        meta_rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            meta_rows,
            vec![("Assets/Game/Canonical.prefab".to_string(), 10)]
        );
    }

    #[test]
    fn delete_missing_asset_path_preserves_incoming_missing_reference_edges() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let src = test_asset("Assets/Scenes/Main.prefab", AssetKind::Prefab, "");
        let dst = test_script_asset(
            "Assets/Scripts/MissingBehaviour.cs",
            "MissingBehaviour",
            "",
            "missingbehaviour monobehaviour",
            "monobehaviour",
        );
        seed_assets(&mut conn, &[src.clone(), dst.clone()]);

        let tx = conn.transaction().unwrap();
        batch_insert_edges(
            &tx,
            &[RefEdge {
                src_guid: src.guid,
                dst_guid: dst.guid,
                dst_file_id: Some(11500000),
                class_id_hint: Some(114),
                field_hint: Some("m_Script".to_string()),
                ref_path: Some("m_Component[0].component".to_string()),
            }],
        )
        .unwrap();
        tx.commit().unwrap();

        let removed =
            delete_missing_asset_path(&mut conn, "Assets/Scripts/MissingBehaviour.cs").unwrap();
        assert!(removed);

        let counts = get_missing_reference_counts(&conn).unwrap();
        assert_eq!(counts.missing_scripts, 1);
        assert_eq!(counts.broken_references, 0);

        let incoming = get_direct_refs(&conn, &dst.guid).unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].src_guid, src.guid);
    }

    #[test]
    fn atomic_update_asset_removes_stale_same_path_guid() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        let old_guid = parse_guid_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let new_guid = parse_guid_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let path = "Packages/com.farlocus.locus/Editor";
        let meta_path = format!("{}.meta", path);
        let mut stale = test_asset(path, AssetKind::MetaOnly, "");
        stale.guid = old_guid;
        stale.ext = String::new();
        stale.exists_on_disk = false;
        stale.mtime_ns = 1;
        stale.meta_hash = hash128(b"old-meta");
        let target = test_asset("Assets/Scenes/Main.prefab", AssetKind::Prefab, "");
        seed_assets(&mut conn, &[stale.clone(), target.clone()]);

        let tx = conn.transaction().unwrap();
        batch_insert_files(
            &tx,
            &[(
                meta_path.clone(),
                FileRole::Meta,
                1,
                8,
                hash128(b"old-meta"),
                Some(old_guid),
            )],
        )
        .unwrap();
        batch_insert_edges(
            &tx,
            &[RefEdge {
                src_guid: old_guid,
                dst_guid: target.guid,
                dst_file_id: None,
                class_id_hint: None,
                field_hint: None,
                ref_path: None,
            }],
        )
        .unwrap();
        tx.commit().unwrap();

        let mut fresh = stale.clone();
        fresh.guid = new_guid;
        fresh.mtime_ns = 20;
        fresh.meta_hash = hash128(b"new-meta");
        atomic_update_asset(
            &mut conn,
            &fresh,
            &[],
            &[(
                meta_path.clone(),
                FileRole::Meta,
                20,
                8,
                hash128(b"new-meta"),
            )],
        )
        .unwrap();

        assert_eq!(resolve_guid_by_path(&conn, path).unwrap(), Some(new_guid));
        assert_eq!(resolve_path_by_guid(&conn, &old_guid).unwrap(), None);
        assert!(get_direct_deps(&conn, &old_guid).unwrap().is_empty());

        let rows = get_all_asset_mtime_records(&conn).unwrap();
        let same_path_rows = rows
            .iter()
            .filter(|record| record.path == path)
            .collect::<Vec<_>>();
        assert_eq!(same_path_rows.len(), 1);
        assert_eq!(same_path_rows[0].mtime_ns, 20);

        let old_fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM asset_search_fts WHERE guid_hex = ?1",
                rusqlite::params![guid_to_hex(&old_guid)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_fts_count, 0);
    }

    #[test]
    fn parse_query_lenient_keeps_bare_terms_split() {
        let parsed = parse_query_lenient("t:prefab hero enemy under:Assets/UI");
        assert_eq!(parsed.predicates.len(), 2);
        assert_eq!(
            parsed.bare_terms,
            vec!["hero".to_string(), "enemy".to_string()]
        );
    }

    #[test]
    fn parse_query_lenient_supports_quoted_bare_terms() {
        let parsed = parse_query_lenient(r#"t:prefab "hero enemy" under:"Assets/My Folder""#);
        assert_eq!(parsed.predicates.len(), 2);
        assert_eq!(parsed.bare_terms, vec!["hero enemy".to_string()]);

        match &parsed.predicates[1] {
            SearchPredicate::Under(path) => assert_eq!(path, "Assets/My Folder"),
            other => panic!("expected under predicate, got {other:?}"),
        }
    }

    #[test]
    fn search_assets_for_command_uses_and_semantics_for_space_terms() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed_assets(
            &mut conn,
            &[
                test_asset("Assets/UI/HeroEnemy.prefab", AssetKind::Prefab, ""),
                test_asset("Assets/UI/Hero.prefab", AssetKind::Prefab, ""),
                test_asset("Assets/UI/Enemy.prefab", AssetKind::Prefab, ""),
            ],
        );

        let rows =
            search_assets_for_command(&conn, "hero enemy", &[AssetRoot::Assets], 20).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "Assets/UI/HeroEnemy.prefab");
    }

    #[test]
    fn search_assets_for_command_handles_mixed_short_and_long_terms() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed_assets(
            &mut conn,
            &[
                test_asset("Assets/UI/Hero.prefab", AssetKind::Prefab, ""),
                test_asset("Assets/Characters/Hero.prefab", AssetKind::Prefab, ""),
                test_asset("Assets/UI/Villain.prefab", AssetKind::Prefab, ""),
            ],
        );

        let rows = search_assets_for_command(&conn, "ui hero", &[AssetRoot::Assets], 20).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "Assets/UI/Hero.prefab");
    }

    #[test]
    fn find_script_descendant_paths_matches_whole_type_terms_only() {
        let mut conn = Connection::open_in_memory().unwrap();
        let source = test_script_asset(
            "Assets/Plugins/UniTask/Linq/Do.cs",
            "Do",
            "cysharp.threading.tasks.linq",
            "do cysharp.threading.tasks.linq.do object",
            "object",
        );
        let exact = test_script_asset(
            "Assets/Game/CustomYield.cs",
            "CustomYield",
            "cysharp.threading.tasks.linq",
            "customyield cysharp.threading.tasks.linq.customyield do cysharp.threading.tasks.linq.do scriptableobject",
            "do cysharp.threading.tasks.linq.do scriptableobject",
        );
        let same_name_duplicate = test_script_asset(
            "Assets/Plugins/UniTask/Linq/Do.Duplicate.cs",
            "Do",
            "cysharp.threading.tasks.linq",
            "do cysharp.threading.tasks.linq.do object",
            "object",
        );
        let other_namespace_duplicate = test_script_asset(
            "Assets/Plugins/Astar/Core/Do.cs",
            "Do",
            "pathfinding.util",
            "do pathfinding.util.do object",
            "object",
        );
        let substring_only = test_script_asset(
            "Assets/Plugins/UniTask/Runtime/PlayerLoopTimer.cs",
            "PlayerLoopTimer",
            "cysharp.threading.tasks",
            "playerlooptimer cysharp.threading.tasks.playerlooptimer asyncenumerable",
            "asyncenumerable",
        );
        seed_assets(
            &mut conn,
            &[
                source.clone(),
                exact,
                same_name_duplicate,
                other_namespace_duplicate,
                substring_only,
            ],
        );

        let rows = find_script_descendant_paths(
            &conn,
            &[source.script_full_name_lower.clone()],
            &source.guid,
        )
        .unwrap();
        assert_eq!(rows, vec!["Assets/Game/CustomYield.cs".to_string()]);
    }

    #[test]
    fn get_stored_script_metadata_for_base_type_prefers_namespace_and_rejects_ambiguity() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed_assets(
            &mut conn,
            &[
                test_script_asset(
                    "Assets/Game/Combat/BaseNode.cs",
                    "BaseNode",
                    "game.combat",
                    "basenode game.combat.basenode scriptableobject",
                    "scriptableobject",
                ),
                test_script_asset(
                    "Assets/Tools/BaseNode.cs",
                    "BaseNode",
                    "tools.graph",
                    "basenode tools.graph.basenode object",
                    "object",
                ),
            ],
        );

        let preferred =
            get_stored_script_metadata_for_base_type(&conn, "BaseNode", Some("game.combat"))
                .unwrap()
                .expect("same-namespace base should resolve");
        assert_eq!(preferred.full_name_lower, "game.combat.basenode");

        let ambiguous = get_stored_script_metadata_for_base_type(&conn, "BaseNode", None).unwrap();
        assert!(ambiguous.is_none());
    }

    #[test]
    fn search_assets_formats_generic_asset_tp_with_script_class_name() {
        let mut conn = Connection::open_in_memory().unwrap();
        let mut asset = test_asset(
            "Assets/ScriptableObjects/EventChannels/Cure.asset",
            AssetKind::GenericAsset,
            "cureeventchannel scriptableobject",
        );
        asset.script_class_name = Some("CureEventChannel".to_string());
        seed_assets(&mut conn, &[asset]);

        let result = search_assets(
            &conn,
            &[SearchPredicate::Type(vec![AssetKind::GenericAsset])],
            &["tp".to_string(), "p".to_string()],
            20,
            0,
        )
        .unwrap();

        assert_eq!(result.total, 1);
        assert_eq!(
            result.rows[0].tp.as_deref(),
            Some("CureEventChannel(genericAsset)")
        );
        assert_eq!(
            result.rows[0].p.as_deref(),
            Some("Assets/ScriptableObjects/EventChannels/Cure.asset")
        );
    }
}
